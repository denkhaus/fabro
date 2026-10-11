//! Retrying a run: a fork from its last checkpoint.
//!
//! A retry forks a terminal run at its last checkpoint. When the run failed
//! on a stage (its last durable finish is the failed stage's), the position's
//! firing runs again, so the retry reruns the failed stage on the files of
//! the stage before it; a run that succeeded, was cancelled or died forks at
//! the last position as it stands, and continues from there.

use fabro_types::{FailureReason, RunId, RunProjection, RunStatus};

use super::fork::{ensure_forkable, ensure_terminal};
use crate::error::Error;

/// A run can be retried when it is terminal and not archived.
pub fn ensure_retryable(source: &RunProjection, run_id: &RunId) -> Result<(), Error> {
    ensure_forkable(source, run_id)?;
    ensure_terminal(source, run_id, "retry")
}

/// Whether the retry reruns the last checkpointed stage: it does when the
/// run failed on its own terms, since that stage's finish is the failure.
/// A quota-class park reruns it too (fabro-2c4a): the park fired because
/// the provider window closed while the run was in flight, so the last
/// checkpointed stage is the interrupted work — resuming means finishing
/// it, not repeating the whole run.
#[must_use]
pub fn reruns_last(status: RunStatus) -> bool {
    match status {
        RunStatus::Failed { reason } => reason != FailureReason::Cancelled,
        RunStatus::Blocked {
            blocked_reason: fabro_types::BlockedReason::QuotaRateLimit,
        } => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_run_reruns_its_last_stage_and_the_others_continue() {
        assert!(reruns_last(RunStatus::Failed {
            reason: FailureReason::WorkflowError,
        }));
        assert!(!reruns_last(RunStatus::Failed {
            reason: FailureReason::Cancelled,
        }));
        assert!(!reruns_last(RunStatus::Dead));
        // The quota park resumes the interrupted last stage (fabro-2c4a).
        assert!(reruns_last(RunStatus::Blocked {
            blocked_reason: fabro_types::BlockedReason::QuotaRateLimit,
        }));
        assert!(!reruns_last(RunStatus::Blocked {
            blocked_reason: fabro_types::BlockedReason::HumanInputRequired,
        }));
        assert!(!reruns_last(RunStatus::Succeeded {
            reason: fabro_types::SuccessReason::Completed,
        }));
    }
}
