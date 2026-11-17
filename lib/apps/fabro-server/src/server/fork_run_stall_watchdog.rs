//! Fork-only run-stall watchdog (fabro-d4f2, 2026-10-20).
//!
//! A run in `running` whose stream has been silent for over 90 minutes
//! is the wedge class the 2026-10-19 incident exposed: the engine had
//! accepted a `$interrupt` (`attractor.turn.interrupted`) but never
//! resolved the firing — status stayed `running` with zero events for
//! hours, `stall_timeout` did not cover the state, and the loop lane's
//! `on_overlap=skip` starved silently. This watchdog is the NOTICE half
//! of the fix: it WARNs (rootprint-visible) as a run's silence crosses
//! the threshold. WARN only, never an action — user decision on the
//! grill's noul question (2026-10-20): a watchdog that auto-soft-stops
//! can kill legitimate long work; humans and line-watch decide.
//!
//! Crossing window, not every tick: a run warns once as its last event
//! passes the 90-minute mark (last_event_at_ms inside
//! `[now - 90min - interval, now - 90min)`), so a wedged run produces
//! one WARN per episode, not a WARN storm. A server restart re-warns a
//! still-wedged run once — acceptable for a notice-only watchdog.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use tokio::task::JoinHandle;
use tokio::time;
use tracing::{Instrument as _, info, info_span, warn};

use super::AppState;

/// First pass waits out startup reconciliation, mirroring the sibling
/// supervisors: boot-time projections must not read as stalls.
const STALL_WATCHDOG_STARTUP_DELAY: Duration = Duration::from_mins(5);
const STALL_WATCHDOG_INTERVAL: Duration = Duration::from_mins(15);
const STALL_AFTER: Duration = Duration::from_mins(90);

/// Fire-and-forget like the sibling supervisors in `serve.rs`: the loop
/// owns its shutdown through `AppState::shutdown_token()`.
pub(crate) fn spawn_run_stall_watchdog(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(run_stall_watchdog(state.clone()).instrument(info_span!("run_stall_watchdog")));
    tokio::spawn(run_owner_park_watchdog(state).instrument(info_span!("owner_park_watchdog")))
}

async fn run_stall_watchdog(state: Arc<AppState>) {
    let shutdown = state.shutdown_token();
    info!(
        warn_after_mins = STALL_AFTER.as_secs() / 60,
        "run stall watchdog started"
    );

    tokio::select! {
        () = shutdown.cancelled() => return,
        () = time::sleep(STALL_WATCHDOG_STARTUP_DELAY) => {
            stall_watchdog_pass_logged(&state).await;
        }
    }

    let mut ticker = time::interval(STALL_WATCHDOG_INTERVAL);
    ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = ticker.tick() => {
                stall_watchdog_pass_logged(&state).await;
            }
        }
    }
}

/// Fail-open wrapper: a failing pass logs and waits for the next tick —
/// a watchdog must never disturb the server.
async fn stall_watchdog_pass_logged(state: &AppState) {
    if let Err(error) = stall_watchdog_pass(state, Utc::now()).await {
        warn!(%error, "run stall watchdog pass failed");
    }
}

/// The owner-park arm (work#1345, deploy-drift arm style): an owner park
/// PINS its run's work-item claim — an aged park is a stuck item, not a
/// stalled worker. WARN once as a park crosses the age threshold; WARN
/// only, never an action: the park is the owner's own retry-intent
/// signal, and the disposition (or the owner) clears it.
const OWNER_PARK_WARN_AFTER: Duration = Duration::from_hours(24);

async fn run_owner_park_watchdog(state: Arc<AppState>) {
    let shutdown = state.shutdown_token();
    info!(
        warn_after_hours = OWNER_PARK_WARN_AFTER.as_secs() / 3600,
        "owner park watchdog started"
    );

    tokio::select! {
        () = shutdown.cancelled() => return,
        () = time::sleep(STALL_WATCHDOG_STARTUP_DELAY) => {
            owner_park_watchdog_pass_logged(&state).await;
        }
    }

    let mut ticker = time::interval(STALL_WATCHDOG_INTERVAL);
    ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            () = shutdown.cancelled() => break,
            _ = ticker.tick() => {
                owner_park_watchdog_pass_logged(&state).await;
            }
        }
    }
}

async fn owner_park_watchdog_pass_logged(state: &AppState) {
    if let Err(error) = owner_park_watchdog_pass(state, Utc::now()).await {
        warn!(%error, "owner park watchdog pass failed");
    }
}

/// One watchdog pass over the crossing window: every owner-parked run
/// whose park just crossed the [`OWNER_PARK_WARN_AFTER`] threshold WARNs
/// once. `Ok(count)` = runs warned this pass.
pub(super) async fn owner_park_watchdog_pass(
    state: &AppState,
    now: DateTime<Utc>,
) -> anyhow::Result<usize> {
    let threshold = chrono::Duration::from_std(OWNER_PARK_WARN_AFTER)?;
    let window = threshold + chrono::Duration::from_std(STALL_WATCHDOG_INTERVAL)?;
    let aged = state
        .stores
        .run_summaries
        .owner_parked_run_ids_aged_between(now - window, now - threshold)
        .await?;
    for run_id in &aged {
        warn!(
            run_id = %run_id,
            parked_for_hours = OWNER_PARK_WARN_AFTER.as_secs() / 3600,
            "owner park aged: run parked owner-pending past the threshold, its work-item claim \
             is pinned (work#1345 watchdog)"
        );
    }
    Ok(aged.len())
}

/// One watchdog pass over the crossing window: every `running` run whose
/// last event just crossed the [`STALL_AFTER`] threshold WARNs once.
/// `Ok(count)` = runs warned this pass.
pub(super) async fn stall_watchdog_pass(
    state: &AppState,
    now: DateTime<Utc>,
) -> anyhow::Result<usize> {
    let window = chrono::Duration::from_std(STALL_AFTER)?
        + chrono::Duration::from_std(STALL_WATCHDOG_INTERVAL)?;
    let older_than_ms = (now - window).timestamp_millis();
    let younger_than_ms = (now - chrono::Duration::from_std(STALL_AFTER)?).timestamp_millis();
    let stalled = state
        .stores
        .run_summaries
        .stalled_running_run_ids(older_than_ms, younger_than_ms)
        .await?;
    for run_id in &stalled {
        warn!(
            run_id = %run_id,
            silent_for_mins = STALL_AFTER.as_secs() / 60,
            "run stalled: running with no stream events past the threshold (fabro-d4f2 watchdog)"
        );
    }
    Ok(stalled.len())
}
