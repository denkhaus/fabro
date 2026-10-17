//! Fork-only run-retention supervisor (fabro-b1a4, 2026-10-17).
//!
//! After a startup grace window and then every 24h, terminal runs older
//! than `[server.retention].max_run_age_days` (defaults.toml: 14, `0`
//! disables the pass) are pruned through the SAME plan/execute path the
//! manual `fabro system prune` uses (`handler::system::plan_prune` /
//! `execute_prune`), so the age semantics of the CLI and the scheduled
//! pass cannot drift. Run deletion prunes the run's sandbox through
//! Petri's lease ledger — that is what actually reclaims the exited run
//! containers on the host; the run dir and the store row go with it.
//!
//! User decision (2026-10-17): enforcement from the first tick, no
//! report-only phase. Failure posture: FAIL-OPEN — a failing pass logs at
//! ERROR and retries on the next tick; retention is hygiene, never a
//! reason to disturb the server. Each enabled pass logs its plan (run
//! count + bytes) at INFO before deleting (logging-strategy: fixed
//! message, data in fields).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use fabro_api::types::PruneRunsRequest;
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{Instrument as _, info, info_span, warn};

use super::handler::system::{execute_prune, plan_prune};
use super::{AppState, to_i64};

/// First pass waits out startup reconciliation (projector startup_pass,
/// scheduler boot) before any deletion.
const RUN_RETENTION_STARTUP_DELAY: Duration = Duration::from_mins(5);
const RUN_RETENTION_SCAN_INTERVAL: Duration = Duration::from_hours(24);

/// What one executed retention pass reclaimed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RetentionPassStats {
    pub(super) planned:       usize,
    pub(super) deleted_count: usize,
    pub(super) freed_bytes:   i64,
}

/// Fire-and-forget like the sibling supervisors in `serve.rs`: the loop
/// owns its shutdown through `AppState::shutdown_token()` (cancellation
/// pattern, not handle ownership).
pub(crate) fn spawn_run_retention_supervisor(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(run_retention_supervisor(state).instrument(info_span!("run_retention_supervisor")))
}

async fn run_retention_supervisor(state: Arc<AppState>) {
    let shutdown = state.shutdown_token();

    let retention_days = state.server_settings().server.retention.max_run_age_days;
    info!(
        max_run_age_days = retention_days,
        enabled = retention_days > 0,
        "run retention supervisor started"
    );

    tokio::select! {
        () = shutdown.cancelled() => return,
        () = time::sleep(RUN_RETENTION_STARTUP_DELAY) => {
            retention_pass_logged(&state).await;
        }
    }

    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            () = time::sleep(RUN_RETENTION_SCAN_INTERVAL) => {
                retention_pass_logged(&state).await;
            }
        }
    }
}

/// Fail-open wrapper: a failing pass logs and waits for the next tick —
/// retention is hygiene, never a reason to disturb the server.
async fn retention_pass_logged(state: &AppState) {
    if let Err(error) = run_retention_pass(state).await {
        warn!(%error, "run retention pass failed");
    }
}

/// The prune request for a configured age; `None` means retention is
/// disabled (`0`). Kept pure so the fork tests pin the exact request the
/// scheduled pass sends — same `older_than` grammar as the CLI.
pub(super) fn retention_request(max_run_age_days: u64) -> Option<PruneRunsRequest> {
    if max_run_age_days == 0 {
        return None;
    }
    Some(PruneRunsRequest {
        before:     None,
        dry_run:    false,
        labels:     HashMap::default(),
        older_than: Some(format!("{max_run_age_days}d")),
        orphans:    false,
        workflow:   None,
    })
}

/// One retention pass. `Ok(None)` = disabled configuration (no-op);
/// `Ok(Some(stats))` = planned and executed; `Err` = the pass failed and
/// the supervisor logs it (fail-open) until the next tick.
pub(super) async fn run_retention_pass(
    state: &AppState,
) -> anyhow::Result<Option<RetentionPassStats>> {
    let max_run_age_days = state.server_settings().server.retention.max_run_age_days;
    let Some(request) = retention_request(max_run_age_days) else {
        return Ok(None);
    };

    let plan = plan_prune(state, &request)
        .await
        .map_err(|error| anyhow::anyhow!("retention plan failed: {error:?}"))?;
    info!(
        max_run_age_days,
        runs = plan.run_ids.len(),
        bytes = plan.total_size_bytes,
        "run retention plan"
    );
    if plan.run_ids.is_empty() {
        return Ok(Some(RetentionPassStats {
            planned:       0,
            deleted_count: 0,
            freed_bytes:   0,
        }));
    }

    let deleted_count = execute_prune(state, &plan)
        .await
        .map_err(|error| anyhow::anyhow!("retention delete failed: {error:?}"))?;
    info!(
        deleted_runs = deleted_count,
        freed_bytes = plan.total_size_bytes,
        "run retention pass complete"
    );
    Ok(Some(RetentionPassStats {
        planned: plan.run_ids.len(),
        deleted_count,
        freed_bytes: to_i64(plan.total_size_bytes),
    }))
}
