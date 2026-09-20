//! Cross-run circuit breaker for automation schedule triggers (fabro-3d97).
//!
//! An autonomous schedule can fire forever even when every run parks with the
//! same failure (routing defect, provider outage, sandbox issue). The breaker
//! counts consecutive automation-fired terminal runs that park/fail with the
//! SAME failure signature; when the count reaches the trigger's threshold the
//! scheduler pauses the trigger (equivalent to the pause button) and emits one
//! aggregated notification. A succeeded run, a different signature, or a human
//! re-enable resets the count.
//!
//! Signature scheme (one documented scheme, fabro-3d97): the run's existing
//! failure signature (`FailureDetail::signature`, the same concept as the
//! run-level loop breaker, e.g. `api_transient|zai|rate_limited`) is used
//! verbatim when present; otherwise a stable fallback key is derived from the
//! workflow and the park reason as `park|{workflow}|{reason}`.

use chrono::{DateTime, Utc};
use fabro_types::{FailureReason, RunFailure, RunStatus, SuccessReason};
use serde::{Deserialize, Serialize};

/// Default consecutive same-signature failures before a schedule trigger is
/// paused by the breaker.
pub const DEFAULT_SCHEDULE_BREAKER_THRESHOLD: u32 = 3;

/// Persisted breaker facts for one schedule trigger. Counter facts are
/// maintained by the scheduler while the trigger is enabled; `paused_at` is
/// set once when the breaker trips and the trigger is disabled. Re-enabling
/// the trigger (the existing replace path) clears the facts, which resets the
/// counter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleBreakerState {
    /// Failure signature of the current consecutive run.
    pub signature:         String,
    /// Consecutive terminal runs with that signature (0 = clean).
    pub consecutive_count: u32,
    /// Last terminal run processed by the breaker (high-water mark).
    pub last_run_id:       String,
    /// Set when the breaker paused the trigger.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_at:         Option<DateTime<Utc>>,
}

/// Whether a terminal run status counts as a breaker failure. Parked shapes
/// (`Succeeded{Boundary}` park points, `Failed{SoftStop}`, `Failed{Deadlock}`)
/// and ordinary failures count; other successes reset the count.
#[must_use]
pub fn counts_as_breaker_failure(status: RunStatus) -> bool {
    matches!(
        status,
        RunStatus::Failed { .. }
            | RunStatus::Dead
            | RunStatus::Succeeded {
                reason: SuccessReason::Boundary,
            }
    )
}

/// Breaker failure signature for one terminal run. Uses the run's failure
/// detail signature verbatim when present, else the documented
/// `park|{workflow}|{reason}` fallback key.
#[must_use]
pub fn breaker_signature(
    workflow_label: &str,
    status: RunStatus,
    failure: Option<&RunFailure>,
) -> String {
    if let Some(signature) = failure
        .and_then(|failure| failure.detail.signature.as_ref())
        .map(fabro_types::FailureSignature::as_str)
        .filter(|signature| !signature.is_empty())
    {
        return signature.to_string();
    }
    format!("park|{workflow_label}|{}", park_reason_key(status))
}

fn park_reason_key(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Succeeded { .. } => "boundary",
        RunStatus::Failed {
            reason: FailureReason::WorkflowError,
        } => "workflow_error",
        RunStatus::Failed {
            reason: FailureReason::PublishFailed,
        } => "publish_failed",
        RunStatus::Failed {
            reason: FailureReason::Cancelled,
        } => "cancelled",
        RunStatus::Failed {
            reason: FailureReason::ApprovalDenied,
        } => "approval_denied",
        RunStatus::Failed {
            reason: FailureReason::Terminated,
        } => "terminated",
        RunStatus::Failed {
            reason: FailureReason::TransientInfra,
        } => "transient_infra",
        RunStatus::Failed {
            reason: FailureReason::BudgetExhausted,
        } => "budget_exhausted",
        RunStatus::Failed {
            reason: FailureReason::LaunchFailed,
        } => "launch_failed",
        RunStatus::Failed {
            reason: FailureReason::BootstrapFailed,
        } => "bootstrap_failed",
        RunStatus::Failed {
            reason: FailureReason::SandboxInitFailed,
        } => "sandbox_init_failed",
        RunStatus::Failed {
            reason: FailureReason::Deadlock,
        } => "deadlock",
        RunStatus::Failed {
            reason: FailureReason::SoftStop,
        } => "soft_stop",
        RunStatus::Failed {
            reason: FailureReason::ApprovalTimeout,
        } => "approval_timeout",
        RunStatus::Dead => "dead",
        _ => "other",
    }
}

/// Consecutive same-signature failure counter for one schedule trigger.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BreakerCounter {
    signature:         Option<String>,
    consecutive_count: u32,
}

impl BreakerCounter {
    #[must_use]
    pub fn fresh() -> Self {
        Self::default()
    }

    /// Resume counting from persisted facts. Paused facts never resume (the
    /// trigger is disabled), so this only carries live counts.
    #[must_use]
    pub fn from_persisted(state: &ScheduleBreakerState) -> Self {
        Self {
            signature:         Some(state.signature.clone())
                .filter(|_| state.consecutive_count > 0),
            consecutive_count: state.consecutive_count,
        }
    }

    /// Observe one terminal run. A failure with the same signature increments
    /// the count; a failure with a different signature restarts it at one; a
    /// non-failure (success) resets it.
    pub fn observe(&mut self, is_failure: bool, signature: &str) {
        if !is_failure {
            self.signature = None;
            self.consecutive_count = 0;
            return;
        }
        if self.signature.as_deref() == Some(signature) {
            self.consecutive_count += 1;
        } else {
            self.signature = Some(signature.to_string());
            self.consecutive_count = 1;
        }
    }

    #[must_use]
    pub fn consecutive_count(&self) -> u32 {
        self.consecutive_count
    }

    #[must_use]
    pub fn signature(&self) -> Option<&str> {
        self.signature.as_deref()
    }

    /// Whether the counter has reached the pause threshold. Thresholds below
    /// one are treated as one.
    #[must_use]
    pub fn trips_at(&self, threshold: u32) -> bool {
        self.consecutive_count >= threshold.max(1)
    }
}

#[cfg(test)]
mod tests {
    use fabro_types::{FailureCategory, FailureDetail, RunFailure};

    use super::*;

    fn signature(signature: &str) -> RunFailure {
        let mut detail = FailureDetail::new("parked", FailureCategory::Deterministic);
        detail.signature = Some(fabro_types::FailureSignature(signature.to_string()));
        RunFailure {
            reason: fabro_types::FailureReason::SoftStop,
            detail,
        }
    }

    fn soft_stop() -> RunStatus {
        RunStatus::Failed {
            reason: fabro_types::FailureReason::SoftStop,
        }
    }

    fn boundary() -> RunStatus {
        RunStatus::Succeeded {
            reason: SuccessReason::Boundary,
        }
    }

    fn completed() -> RunStatus {
        RunStatus::Succeeded {
            reason: SuccessReason::Completed,
        }
    }

    #[test]
    fn same_signature_failure_reaches_threshold_and_trips() {
        let mut counter = BreakerCounter::fresh();
        for _ in 0..3 {
            counter.observe(true, "api_transient|zai|rate_limited");
        }
        assert_eq!(counter.consecutive_count(), 3);
        assert!(counter.trips_at(DEFAULT_SCHEDULE_BREAKER_THRESHOLD));
    }

    #[test]
    fn different_signature_resets_the_count() {
        let mut counter = BreakerCounter::fresh();
        counter.observe(true, "api_transient|zai|rate_limited");
        counter.observe(true, "api_transient|zai|rate_limited");
        counter.observe(true, "work|deterministic|acp");
        assert_eq!(counter.consecutive_count(), 1);
        assert!(!counter.trips_at(DEFAULT_SCHEDULE_BREAKER_THRESHOLD));
        assert_eq!(counter.signature(), Some("work|deterministic|acp"));
    }

    #[test]
    fn success_resets_the_count() {
        let mut counter = BreakerCounter::fresh();
        counter.observe(true, "api_transient|zai|rate_limited");
        counter.observe(true, "api_transient|zai|rate_limited");
        counter.observe(false, "");
        assert_eq!(counter.consecutive_count(), 0);
        assert_eq!(counter.signature(), None);
    }

    #[test]
    fn threshold_override_trips_earlier_and_later() {
        let mut counter = BreakerCounter::fresh();
        counter.observe(true, "sig");
        assert!(counter.trips_at(1));
        assert!(!counter.trips_at(2));

        let mut slower = BreakerCounter::fresh();
        for _ in 0..5 {
            slower.observe(true, "sig");
        }
        assert!(slower.trips_at(5));
        assert!(slower.trips_at(1));
    }

    #[test]
    fn resume_from_persisted_facts_continues_the_count() {
        let state = ScheduleBreakerState {
            signature:         "api_transient|zai|rate_limited".to_string(),
            consecutive_count: 2,
            last_run_id:       "run-2".to_string(),
            paused_at:         None,
        };
        let mut counter = BreakerCounter::from_persisted(&state);
        assert_eq!(counter.consecutive_count(), 2);
        counter.observe(true, "api_transient|zai|rate_limited");
        assert!(counter.trips_at(DEFAULT_SCHEDULE_BREAKER_THRESHOLD));
    }

    #[test]
    fn persisted_zero_count_resumes_without_a_signature() {
        let state = ScheduleBreakerState {
            signature:         "old".to_string(),
            consecutive_count: 0,
            last_run_id:       "run-9".to_string(),
            paused_at:         None,
        };
        let mut counter = BreakerCounter::from_persisted(&state);
        counter.observe(true, "new");
        assert_eq!(counter.consecutive_count(), 1);
        assert_eq!(counter.signature(), Some("new"));
    }

    #[test]
    fn parked_and_failed_statuses_count_as_failures() {
        assert!(counts_as_breaker_failure(soft_stop()));
        assert!(counts_as_breaker_failure(boundary()));
        assert!(counts_as_breaker_failure(RunStatus::Dead));
        assert!(!counts_as_breaker_failure(completed()));
    }

    #[test]
    fn signature_uses_failure_detail_signature_verbatim() {
        let failure = signature("api_transient|zai|rate_limited");
        assert_eq!(
            breaker_signature("conductor", soft_stop(), Some(&failure)),
            "api_transient|zai|rate_limited"
        );
    }

    #[test]
    fn signature_falls_back_to_workflow_and_park_reason() {
        assert_eq!(
            breaker_signature("conductor", soft_stop(), None),
            "park|conductor|soft_stop"
        );
        assert_eq!(
            breaker_signature("conductor", boundary(), None),
            "park|conductor|boundary"
        );
        // A present-but-empty detail signature falls back too.
        let empty = RunFailure {
            reason: fabro_types::FailureReason::SoftStop,
            detail: FailureDetail::new("parked", FailureCategory::Deterministic),
        };
        assert_eq!(
            breaker_signature("conductor", soft_stop(), Some(&empty)),
            "park|conductor|soft_stop"
        );
    }
}
