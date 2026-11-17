//! Owner park (work#1345): the retry-intent channel.
//!
//! Dragging a failed run into the blocked column parks it as
//! `Blocked { OwnerPending }` — a user-driven lifecycle transition, like
//! archive. While parked the run is NON-terminal, so its work-item claim
//! holds (the work reconciler releases nothing, and the picker's
//! fail-closed claim check refuses another run the item). The park clears
//! on disposition: retry, retry-on-latest and archive restore the
//! terminal verdict the park deferred, and the normal terminal flow —
//! verdict-driven claim release or the successor's claim handoff —
//! resumes.

use fabro_store::platform_records::{RunLifecycleKind, RunLifecycleRecord};
use fabro_types::{BlockedReason, RunId, RunProjection, RunStatus};

use crate::error::Error;

/// The owner-park status.
#[must_use]
pub fn owner_parked_status() -> RunStatus {
    RunStatus::Blocked {
        blocked_reason: BlockedReason::OwnerPending,
    }
}

/// Whether the projection is owner-parked.
#[must_use]
pub fn is_owner_parked(status: RunStatus) -> bool {
    matches!(status, RunStatus::Blocked {
        blocked_reason: BlockedReason::OwnerPending,
    })
}

/// The park gesture is valid from a terminal FAILED or HUMAN-NEEDED run
/// that is not archived — exactly the retry-intent terminals. A succeeded
/// or dead run has no retry intent to signal.
pub fn ensure_parkable(source: &RunProjection, run_id: &RunId) -> Result<(), Error> {
    super::ensure_not_archived(source.archived_at.is_some(), run_id)?;
    match source.status {
        RunStatus::Failed { .. } | RunStatus::HumanNeeded => Ok(()),
        status => Err(Error::Precondition(format!(
            "run {run_id} must be failed or human_needed to owner-park; current status is \
             {status}"
        ))),
    }
}

/// The park record: a `blocked` lifecycle transition to the owner-park
/// status, folded by the projector against the transition matrix.
#[must_use]
pub fn park_record() -> RunLifecycleRecord {
    let mut record = RunLifecycleRecord::new(RunLifecycleKind::Blocked);
    record.status = Some(owner_parked_status());
    record.source = Some("owner_park".to_string());
    record
}

/// The terminal verdict the park deferred, for the restore record: the
/// conclusion's failure names the FAILED reason; a human-needed park has
/// no failure. `None` when the projection carries no conclusion to
/// restore from — a terminal run always has one; `None` is a refusal.
#[must_use]
pub fn restored_status(source: &RunProjection) -> Option<RunStatus> {
    source.conclusion.as_ref().map(|conclusion| {
        conclusion
            .failure
            .as_ref()
            .map_or(RunStatus::HumanNeeded, |failure| RunStatus::Failed {
                reason: failure.reason,
            })
    })
}

/// The unpark precondition: the run must be owner-parked and restorable.
pub fn ensure_unparkable(source: &RunProjection, run_id: &RunId) -> Result<RunStatus, Error> {
    if !is_owner_parked(source.status) {
        return Err(Error::Precondition(format!(
            "run {run_id} is not owner-parked; current status is {}",
            source.status
        )));
    }
    restored_status(source).ok_or_else(|| {
        Error::Precondition(format!(
            "run {run_id} has no conclusion to restore its parked terminal status from"
        ))
    })
}

/// The restore record: a `failed` lifecycle transition back to the
/// deferred terminal verdict. `None` when the run is not owner-parked
/// (or has no conclusion to restore).
#[must_use]
pub fn unpark_record(source: &RunProjection) -> Option<RunLifecycleRecord> {
    if !is_owner_parked(source.status) {
        return None;
    }
    let restored = restored_status(source)?;
    let mut record = RunLifecycleRecord::new(RunLifecycleKind::Failed);
    record.status = Some(restored);
    record.source = Some("owner_unpark".to_string());
    Some(record)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use fabro_types::outcome::StageOutcome;
    use fabro_types::{
        Conclusion, FailureReason, RunFailure, RunStatus, test_support as types_test_support,
    };

    use super::*;

    fn projection(status: RunStatus, conclusion: Option<Conclusion>) -> RunProjection {
        let mut projection = RunProjection::new(
            "owner park test".to_string(),
            types_test_support::test_run_spec(),
            Utc::now(),
        );
        projection.status = status;
        projection.conclusion = conclusion;
        projection
    }

    fn failed_conclusion() -> Conclusion {
        Conclusion {
            timestamp:            Utc::now(),
            status:               StageOutcome::Failed {
                retry_requested: false,
            },
            timing:               fabro_types::RunTiming::default(),
            failure:              Some(RunFailure {
                reason: FailureReason::WorkflowError,
                detail: fabro_types::outcome::FailureDetail::new(
                    "boom",
                    fabro_types::outcome::FailureCategory::Deterministic,
                ),
            }),
            final_git_commit_sha: None,
            stages:               Vec::new(),
            usage:                None,
            diff:                 fabro_types::RunDiff::default(),
            total_retries:        0,
        }
    }

    fn failed() -> RunStatus {
        RunStatus::Failed {
            reason: FailureReason::WorkflowError,
        }
    }

    fn parked() -> RunStatus {
        owner_parked_status()
    }

    #[test]
    fn a_failed_or_human_needed_run_is_parkable_and_nothing_else_is() {
        let run_id = RunId::new();
        let failed_run = projection(failed(), Some(failed_conclusion()));
        assert!(ensure_parkable(&failed_run, &run_id).is_ok());
        assert!(ensure_parkable(&projection(RunStatus::HumanNeeded, None), &run_id).is_ok());
        assert!(
            ensure_parkable(
                &projection(
                    RunStatus::Succeeded {
                        reason: fabro_types::SuccessReason::Completed,
                    },
                    None,
                ),
                &run_id,
            )
            .is_err()
        );
        assert!(ensure_parkable(&projection(parked(), None), &run_id).is_err());
    }

    #[test]
    fn the_park_record_targets_the_owner_pending_status() {
        let record = park_record();
        assert_eq!(record.transition, RunLifecycleKind::Blocked);
        assert_eq!(record.status, Some(parked()));
        assert_eq!(record.source.as_deref(), Some("owner_park"));
    }

    #[test]
    fn the_restore_record_returns_the_deferred_failure_reason() {
        let mut parked_run = projection(parked(), Some(failed_conclusion()));
        parked_run.status = parked();
        let restored = ensure_unparkable(&parked_run, &RunId::new()).expect("restorable");
        assert_eq!(restored, failed());
        let record = unpark_record(&parked_run).expect("restore record");
        assert_eq!(record.transition, RunLifecycleKind::Failed);
        assert_eq!(record.status, Some(failed()));
    }

    #[test]
    fn a_human_needed_park_restores_to_human_needed() {
        let mut parked_run = projection(parked(), None);
        parked_run.conclusion = Some(Conclusion {
            timestamp:            Utc::now(),
            status:               StageOutcome::Failed {
                retry_requested: false,
            },
            timing:               fabro_types::RunTiming::default(),
            failure:              None,
            final_git_commit_sha: None,
            stages:               Vec::new(),
            usage:                None,
            diff:                 fabro_types::RunDiff::default(),
            total_retries:        0,
        });
        assert_eq!(restored_status(&parked_run), Some(RunStatus::HumanNeeded));
    }

    #[test]
    fn an_unparkable_refusal_names_the_current_status() {
        let not_parked = projection(failed(), Some(failed_conclusion()));
        let error = ensure_unparkable(&not_parked, &RunId::new()).unwrap_err();
        assert!(error.to_string().contains("not owner-parked"));
        // The record layer refuses the same way: a non-parked run with a
        // failed conclusion has a restorable status but NO restore record
        // (pinned after a stale-binary near-miss made the handler accept
        // it).
        assert!(unpark_record(&not_parked).is_none());
        // A park without a conclusion is unrestorable: no terminal verdict
        // to restore.
        let mut parked_run = projection(parked(), None);
        parked_run.conclusion = None;
        assert!(ensure_unparkable(&parked_run, &RunId::new()).is_err());
        assert!(unpark_record(&parked_run).is_none());
    }
}
