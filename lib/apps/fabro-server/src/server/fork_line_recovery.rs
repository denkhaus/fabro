//! Fork-only line recovery: fixed 10-minute recheck probes for
//! quota-parked automations (fabro-986b, user decision 2026-09-14).
//!
//! Yesterday's failure mode: a zai usage-window hard cut parked every
//! pass, the schedule breaker latched (or a human paused the trigger),
//! and the line stayed down until an EXTERNAL re-enable. This module
//! makes recovery server-autonomous:
//!
//! - Quota-class parks (SoftStop + TransientInfra + `rate_limit` signature)
//!   never count toward the schedule breaker — the breaker stays armed for real
//!   defects.
//! - While the newest automation-fired terminal run is a quota park, the
//!   scheduler fires a RECHECK PROBE through the normal scheduled-fire path
//!   every 10 minutes. Probes serialize via on_overlap=skip like every fire; a
//!   probe that survives continues the line, a probe that parks again keeps the
//!   cadence. No external trigger, no manual flip.
//!
//! The cadence is deliberately a FIXED interval: the provider's prose
//! reset time is never parsed for timing (user decision 2026-09-14 —
//! zai sends offset-less wallclocks; a fixed probe is simple and
//! observable). Presence pin: `fork_line_recovery_tests.rs` (workflow
//! crate) + the breaker exemption test in this crate's fork test file.
//!
//! Fork-file policy: this file exists only on our fork; upstream does
//! not have it, so no merge can conflict it away.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use fabro_types::outcome::FailureCategory;
use fabro_types::{RunFailure, RunStatus};

use super::AppState;

/// Recheck probe cadence (user decision 2026-09-14: "fixe rechecks alle
/// 10 Minuten").
pub(crate) const RECHECK_INTERVAL_SECS: i64 = 600;

/// Whether a terminal run is a quota-class park the recheck owns.
///
/// Structural detection only: a SoftStop failure with a TransientInfra
/// category and a `rate_limit` signature detail (spelled `rate_limit` or
/// `rate_limited` depending on provider). The message prose is never
/// parsed here.
#[must_use]
pub(crate) fn is_quota_park(status: RunStatus, failure: Option<&RunFailure>) -> bool {
    if !matches!(status, RunStatus::Failed {
        reason: fabro_types::FailureReason::SoftStop,
    }) {
        return false;
    }
    let Some(failure) = failure else {
        return false;
    };
    if failure.detail.category != FailureCategory::TransientInfra {
        return false;
    }
    failure
        .detail
        .signature
        .as_ref()
        .is_some_and(|signature| signature.as_str().contains("rate_limit"))
}

/// In-memory recheck bookkeeping (derived, not persisted).
///
/// `last_probe` gates the cadence per automation; a server restart loses
/// it, which may fire one early probe — harmless, the probe serializes
/// like any fire and a still-closed window just parks again.
#[derive(Default)]
pub(crate) struct RecheckState {
    last_probe: HashMap<String, DateTime<Utc>>,
}

impl RecheckState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Whether a recheck probe is due for `automation_id` whose newest
    /// terminal run is described by (`run_id`, `status`, `failure`).
    ///
    /// The first probe of a park episode fires on the next tick (the
    /// park's true age is unknown without event history; an early probe
    /// against a still-closed window parks again and starts the cadence).
    #[must_use]
    pub(crate) fn recheck_due(
        &mut self,
        automation_id: &str,
        status: RunStatus,
        failure: Option<&RunFailure>,
        now: DateTime<Utc>,
    ) -> bool {
        if !is_quota_park(status, failure) {
            self.last_probe.remove(automation_id);
            return false;
        }
        let Some(last_probe) = self.last_probe.get(automation_id) else {
            return true;
        };
        now.signed_duration_since(*last_probe).num_seconds() >= RECHECK_INTERVAL_SECS
    }

    /// Record that a probe fired for `automation_id`.
    pub(crate) fn note_probe(&mut self, automation_id: &str, now: DateTime<Utc>) {
        self.last_probe.insert(automation_id.to_string(), now);
    }
}

/// Fire one recheck probe per quota-parked automation when due.
///
/// Pure orchestration over [`RecheckState::recheck_due`]: loads the newest
/// terminal automation-fired run, loads its terminal failure (same reader
/// the breaker uses), skips when active, and spawns the probe through the
/// normal scheduled-fire path (on_overlap serializes; materialization and
/// environment resolution are identical to a cron fire).
pub(crate) async fn fire_due_rechecks(
    state: Arc<AppState>,
    automations: &[fabro_automation::Automation],
    now: DateTime<Utc>,
    rechecks: &mut RecheckState,
) {
    for automation in automations {
        // Probes belong to scheduled lines: a manual-only automation is
        // not ours to revive.
        let Some(trigger) = automation.enabled_schedule_triggers().next().cloned() else {
            // Manual-only automations are not ours to revive.
            continue;
        };
        let newest = match state
            .stores
            .run_summaries
            .list_terminal_for_automation(automation.id.as_str(), 1, now)
            .await
        {
            Ok(runs) => runs.into_iter().next(),
            Err(error) => {
                tracing::warn!(
                    automation_id = %automation.id,
                    ?error,
                    "line recheck could not list terminal runs; skipping this tick"
                );
                continue;
            }
        };
        let Some(newest) = newest else {
            continue;
        };
        if state
            .stores
            .run_summaries
            .active_run_for_automation(automation.id.as_str())
            .await
            .ok()
            .flatten()
            .is_some()
        {
            // A live run (or live child) owns the line; on_overlap would
            // skip the probe anyway — do not even spawn it.
            continue;
        }
        let failure = super::automation_breaker::terminal_failure(&state, &newest).await;
        if !rechecks.recheck_due(
            automation.id.as_str(),
            newest.lifecycle.status,
            failure.as_ref(),
            now,
        ) {
            continue;
        }
        rechecks.note_probe(automation.id.as_str(), now);
        let trigger_id = trigger.id.clone();
        tracing::info!(
            automation_id = %automation.id,
            parked_run_id = %newest.id,
            interval_secs = RECHECK_INTERVAL_SECS,
            "line recheck probe firing (fabro-986b fixed 10m recheck)"
        );
        tokio::spawn(super::automation_scheduler::fire_scheduled_automation_run(
            Arc::clone(&state),
            automation.clone(),
            trigger_id,
            now,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn soft_stop() -> RunStatus {
        RunStatus::Failed {
            reason: fabro_types::FailureReason::SoftStop,
        }
    }

    fn quota_failure() -> RunFailure {
        let mut detail = fabro_types::outcome::FailureDetail::new(
            "LLM error: provider zai Usage limit reached for 5 hour",
            FailureCategory::TransientInfra,
        );
        detail.signature = Some(fabro_types::failure_signature::FailureSignature(
            "api_transient|zai|rate_limit".to_string(),
        ));
        RunFailure {
            reason: fabro_types::FailureReason::SoftStop,
            detail,
        }
    }

    #[test]
    fn soft_stop_with_quota_signature_is_a_quota_park() {
        assert!(is_quota_park(soft_stop(), Some(&quota_failure())));
    }

    #[test]
    fn soft_stop_without_quota_signature_is_not_a_quota_park() {
        let mut failure = quota_failure();
        failure.detail.signature = None;
        assert!(!is_quota_park(soft_stop(), Some(&failure)));
    }

    #[test]
    fn hard_failures_are_not_quota_parks() {
        let hard = RunStatus::Failed {
            reason: fabro_types::FailureReason::WorkflowError,
        };
        assert!(!is_quota_park(hard, Some(&quota_failure())));
        assert!(!is_quota_park(soft_stop(), None));
    }

    #[test]
    fn recheck_cadence_is_ten_minutes_from_last_probe() {
        let mut state = RecheckState::new();
        let failure = quota_failure();
        let t0 = Utc::now();

        assert!(state.recheck_due("a", soft_stop(), Some(&failure), t0));
        state.note_probe("a", t0);

        assert!(!state.recheck_due("a", soft_stop(), Some(&failure), t0));
        assert!(state.recheck_due(
            "a",
            soft_stop(),
            Some(&failure),
            t0 + chrono::Duration::seconds(RECHECK_INTERVAL_SECS)
        ));
    }

    #[test]
    fn recovered_automation_resets_its_probe_gate() {
        let mut state = RecheckState::new();
        let failure = quota_failure();
        let t0 = Utc::now();
        state.note_probe("a", t0);

        let recovered = RunStatus::Succeeded {
            reason: fabro_types::SuccessReason::Completed,
        };
        assert!(!state.recheck_due("a", recovered, Some(&failure), t0));
        assert!(
            !state.last_probe.contains_key("a"),
            "a recovered automation forgets its probe gate"
        );
    }

    #[test]
    fn recheck_interval_is_ten_minutes() {
        assert_eq!(RECHECK_INTERVAL_SECS, 600, "user decision 2026-09-14");
    }
}
