//! Shared run-lifecycle transition table (fabro-3fe4).
//!
//! One pure function owns the mapping `(RunStatus, EventBody) -> RunStatus`
//! for every event that drives run status. Both reducers — the server's live
//! in-memory reducer (`update_live_run_from_event`) and the durable
//! projection reducer (`fabro_store::RunProjectionReducer`) — consume this
//! table so their status folds cannot drift. Reducers keep only their
//! non-status side effects on top of it.

use crate::run_event::EventBody;
use crate::status::{InvalidTransition, RunStatus};

/// Result of applying one event body to a run status.
#[derive(Debug, Clone, PartialEq)]
pub enum LifecycleTransition {
    /// The event does not participate in the run-lifecycle state machine;
    /// reducers leave the status untouched.
    NotLifecycle,
    /// The event is valid for the current status; `RunStatus` is the status
    /// after the event (identical to the input for idempotent repeats).
    Next(RunStatus),
    /// The event cannot follow the current status.
    Rejected(InvalidTransition),
}

/// Pure transition table for the run-lifecycle event protocol.
///
/// Validity reuses [`RunStatus::can_transition_to`]; an event whose target
/// equals the current status is an idempotent no-op (`Next(current)`),
/// mirroring `RunProjection::try_apply_status`.
///
/// Protocol notes:
/// - `run.submitted` carries the definition blob but does not drive status: the
///   resubmit flow reaches `Submitted` through `run.start_requested` (`resume:
///   true`), and a status write here would force every later `run.starting`
///   through the historic `Submitted -> Starting` replay shim.
/// - `run.start_requested` with `resume: false` is a request marker only; the
///   status change is owned by the following `run.runnable` event.
#[must_use]
pub fn apply_lifecycle_event(status: RunStatus, body: &EventBody) -> LifecycleTransition {
    let target = match body {
        EventBody::RunStartRequested(props) if props.resume => Some(RunStatus::Submitted),
        EventBody::RunPending(props) => Some(RunStatus::Pending {
            reason: props.reason,
        }),
        EventBody::RunRunnable(_) => Some(RunStatus::Runnable),
        EventBody::RunStarting(_) => Some(RunStatus::Starting),
        EventBody::RunRunning(_) => Some(RunStatus::Running),
        EventBody::RunBlocked(props) => Some(match status {
            RunStatus::Paused { .. } => RunStatus::Paused {
                prior_block: Some(props.blocked_reason),
            },
            _ => RunStatus::Blocked {
                blocked_reason: props.blocked_reason,
            },
        }),
        EventBody::RunUnblocked(_) => Some(match status {
            RunStatus::Paused { .. } => RunStatus::Paused { prior_block: None },
            _ => RunStatus::Running,
        }),
        EventBody::RunRemoving(_) => Some(RunStatus::Removing),
        EventBody::RunPaused(_) => Some(RunStatus::Paused {
            prior_block: status.blocked_reason(),
        }),
        EventBody::RunUnpaused(_) => Some(match status {
            RunStatus::Paused {
                prior_block: Some(blocked_reason),
            } => RunStatus::Blocked { blocked_reason },
            _ => RunStatus::Running,
        }),
        EventBody::RunCompleted(props) => Some(RunStatus::Succeeded {
            reason: props.reason,
        }),
        EventBody::RunFailed(props) => Some(RunStatus::Failed {
            reason: props.failure.reason,
        }),
        _ => None,
    };

    match target {
        None => LifecycleTransition::NotLifecycle,
        Some(next) if next == status => LifecycleTransition::Next(next),
        Some(next) => match status.transition_to(next) {
            Ok(next) => LifecycleTransition::Next(next),
            Err(err) => LifecycleTransition::Rejected(err),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_event::{
        RunApprovedProps, RunBlockedProps, RunCompletedProps, RunControlEffectProps,
        RunCreatedProps, RunFailedProps, RunPendingProps, RunRunnableProps, RunStartRequestedProps,
        RunStatusEffectProps, RunStatusTransitionProps, RunSubmittedProps,
    };
    use crate::{
        BlockedReason, FailureCategory, FailureDetail, FailureReason, Graph, PendingReason,
        RunFailure, RunRunnableSource, RunTiming, SuccessReason, WorkflowSettings, test_support,
    };

    fn transition_props() -> RunStatusTransitionProps {
        RunStatusTransitionProps {}
    }

    fn effect() -> RunControlEffectProps {
        RunControlEffectProps {}
    }

    fn unblocked() -> RunStatusEffectProps {
        RunStatusEffectProps {}
    }

    fn failed(reason: FailureReason) -> RunFailedProps {
        RunFailedProps {
            failure:              RunFailure {
                reason,
                detail: FailureDetail::new("table test", FailureCategory::Deterministic),
            },
            timing:               RunTiming::default(),
            final_git_commit_sha: None,
            final_patch:          None,
            diff_summary:         None,
            billing:              None,
        }
    }

    fn completed(reason: SuccessReason) -> RunCompletedProps {
        RunCompletedProps {
            timing: RunTiming::wall_only(1),
            artifact_count: 0,
            status: "succeeded".to_string(),
            reason,
            failure: None,
            total_usd_micros: None,
            final_git_commit_sha: None,
            final_patch: None,
            diff_summary: None,
            billing: None,
        }
    }

    fn blocked() -> RunBlockedProps {
        RunBlockedProps {
            blocked_reason: BlockedReason::HumanInputRequired,
        }
    }

    fn next(status: RunStatus, body: &EventBody) -> RunStatus {
        match apply_lifecycle_event(status, body) {
            LifecycleTransition::Next(next) => next,
            other => panic!("expected Next, got {other:?} for {status} + {body:?}"),
        }
    }

    fn rejected(status: RunStatus, body: &EventBody) {
        assert!(
            matches!(
                apply_lifecycle_event(status, body),
                LifecycleTransition::Rejected(_)
            ),
            "expected Rejected for {status} + {body:?}"
        );
    }

    #[test]
    fn request_start_resubmits_terminal_runs_to_submitted() {
        let resume = EventBody::RunStartRequested(RunStartRequestedProps { resume: true });
        assert_eq!(
            next(
                RunStatus::Failed {
                    reason: FailureReason::WorkflowError,
                },
                &resume
            ),
            RunStatus::Submitted
        );
        // Fresh (non-resume) start requests are markers only.
        let fresh = EventBody::RunStartRequested(RunStartRequestedProps { resume: false });
        assert_eq!(
            apply_lifecycle_event(RunStatus::Submitted, &fresh),
            LifecycleTransition::NotLifecycle
        );
    }

    #[test]
    fn runnable_pending_and_starting_advance_in_order() {
        let pending = EventBody::RunPending(RunPendingProps {
            reason: PendingReason::ApprovalRequired,
        });
        assert_eq!(next(RunStatus::Submitted, &pending), RunStatus::Pending {
            reason: PendingReason::ApprovalRequired,
        });
        assert_eq!(
            next(
                RunStatus::Pending {
                    reason: PendingReason::ApprovalRequired,
                },
                &pending
            ),
            RunStatus::Pending {
                reason: PendingReason::ApprovalRequired,
            }
        );

        let runnable = EventBody::RunRunnable(RunRunnableProps {
            source: RunRunnableSource::StartRequested,
        });
        assert_eq!(next(RunStatus::Submitted, &runnable), RunStatus::Runnable);
        assert_eq!(
            next(
                RunStatus::Pending {
                    reason: PendingReason::ApprovalRequired,
                },
                &runnable
            ),
            RunStatus::Runnable
        );

        let starting = EventBody::RunStarting(transition_props());
        assert_eq!(next(RunStatus::Runnable, &starting), RunStatus::Starting);
        // Cancel-before-starting: Submitted and Pending runs can be cancelled
        // before any worker reaches Starting.
        assert_eq!(
            next(
                RunStatus::Submitted,
                &EventBody::RunFailed(failed(FailureReason::Cancelled))
            ),
            RunStatus::Failed {
                reason: FailureReason::Cancelled,
            }
        );
        assert_eq!(
            next(
                RunStatus::Pending {
                    reason: PendingReason::ApprovalRequired,
                },
                &EventBody::RunFailed(failed(FailureReason::ApprovalTimeout))
            ),
            RunStatus::Failed {
                reason: FailureReason::ApprovalTimeout,
            }
        );
    }

    #[test]
    fn submitted_cannot_skip_runnable_to_starting() {
        rejected(
            RunStatus::Submitted,
            &EventBody::RunStarting(transition_props()),
        );
        rejected(
            RunStatus::Submitted,
            &EventBody::RunRunning(transition_props()),
        );
    }

    #[test]
    fn failure_before_running_is_valid_only_for_pre_start_reasons() {
        // Failure-before-running: a worker that dies after being scheduled
        // (Runnable) but before appending RunStarting fails the run directly.
        assert_eq!(
            next(
                RunStatus::Runnable,
                &EventBody::RunFailed(failed(FailureReason::LaunchFailed))
            ),
            RunStatus::Failed {
                reason: FailureReason::LaunchFailed,
            }
        );
        assert_eq!(
            next(
                RunStatus::Runnable,
                &EventBody::RunFailed(failed(FailureReason::Cancelled))
            ),
            RunStatus::Failed {
                reason: FailureReason::Cancelled,
            }
        );
        // Reasons that imply the run already progressed past Starting are
        // rejected from Runnable.
        rejected(
            RunStatus::Runnable,
            &EventBody::RunFailed(failed(FailureReason::SandboxInitFailed)),
        );
        rejected(
            RunStatus::Runnable,
            &EventBody::RunFailed(failed(FailureReason::PublishFailed)),
        );
    }

    #[test]
    fn running_blocked_pause_and_control_round_trips() {
        let starting = EventBody::RunStarting(transition_props());
        let running = EventBody::RunRunning(transition_props());
        let runnable = RunStatus::Runnable;
        assert_eq!(next(runnable, &starting), RunStatus::Starting);
        assert_eq!(next(RunStatus::Starting, &running), RunStatus::Running);

        assert_eq!(
            next(RunStatus::Running, &EventBody::RunBlocked(blocked())),
            RunStatus::Blocked {
                blocked_reason: BlockedReason::HumanInputRequired,
            }
        );
        assert_eq!(
            next(
                RunStatus::Blocked {
                    blocked_reason: BlockedReason::HumanInputRequired,
                },
                &EventBody::RunRunning(transition_props())
            ),
            RunStatus::Running
        );

        // Pause preserves the prior block; unpaused restores it.
        assert_eq!(
            next(
                RunStatus::Blocked {
                    blocked_reason: BlockedReason::HumanInputRequired,
                },
                &EventBody::RunPaused(effect())
            ),
            RunStatus::Paused {
                prior_block: Some(BlockedReason::HumanInputRequired),
            }
        );
        assert_eq!(
            next(
                RunStatus::Paused {
                    prior_block: Some(BlockedReason::HumanInputRequired),
                },
                &EventBody::RunUnpaused(effect())
            ),
            RunStatus::Blocked {
                blocked_reason: BlockedReason::HumanInputRequired,
            }
        );

        // Unblocked clears a prior block while paused, otherwise resumes.
        assert_eq!(
            next(
                RunStatus::Paused {
                    prior_block: Some(BlockedReason::HumanInputRequired),
                },
                &EventBody::RunUnblocked(unblocked())
            ),
            RunStatus::Paused { prior_block: None }
        );
        assert_eq!(
            next(
                RunStatus::Blocked {
                    blocked_reason: BlockedReason::HumanInputRequired,
                },
                &EventBody::RunUnblocked(unblocked())
            ),
            RunStatus::Running
        );

        // A block arriving while paused refreshes the prior block instead of
        // leaving the paused state.
        assert_eq!(
            next(
                RunStatus::Paused { prior_block: None },
                &EventBody::RunBlocked(blocked())
            ),
            RunStatus::Paused {
                prior_block: Some(BlockedReason::HumanInputRequired),
            }
        );
    }

    #[test]
    fn terminal_events_close_active_runs_and_stay_frozen() {
        let running = EventBody::RunRunning(transition_props());
        let succeeded = EventBody::RunCompleted(completed(SuccessReason::Completed));
        assert_eq!(next(RunStatus::Running, &succeeded), RunStatus::Succeeded {
            reason: SuccessReason::Completed,
        });
        assert_eq!(
            next(
                RunStatus::Starting,
                &EventBody::RunFailed(failed(FailureReason::Terminated))
            ),
            RunStatus::Failed {
                reason: FailureReason::Terminated,
            }
        );

        // Terminal statuses are immutable: no lifecycle event moves them
        // (Dead remains the deletion escape hatch outside this table's
        // lifecycle events).
        let terminal = RunStatus::Succeeded {
            reason: SuccessReason::Completed,
        };
        rejected(terminal, &running);
        rejected(terminal, &EventBody::RunStarting(transition_props()));
        rejected(
            RunStatus::Failed {
                reason: FailureReason::WorkflowError,
            },
            &EventBody::RunCompleted(completed(SuccessReason::Completed)),
        );
    }

    #[test]
    fn non_lifecycle_events_leave_status_untouched() {
        for body in [
            EventBody::RunCreated(RunCreatedProps {
                title:               None,
                settings:            WorkflowSettings::default(),
                graph:               Graph::new("test"),
                workflow_source:     None,
                labels:              std::collections::BTreeMap::new(),
                source_directory:    None,
                workflow_slug:       None,
                workflow_version_id: None,
                target:              None,
                automation:          None,
                provenance:          test_support::test_run_provenance(),
                manifest_blob:       None,
                spec_blob:           None,
                git:                 None,
                fork_source_ref:     None,
                retried_from:        None,
                parent_id:           None,
                web_url:             None,
            }),
            EventBody::RunSubmitted(RunSubmittedProps {
                definition_blob: None,
            }),
            EventBody::RunApproved(RunApprovedProps {}),
        ] {
            assert_eq!(
                apply_lifecycle_event(RunStatus::Running, &body),
                LifecycleTransition::NotLifecycle,
                "{body:?}"
            );
        }
    }

    #[test]
    fn idempotent_repeats_return_current_status() {
        let running = EventBody::RunRunning(transition_props());
        assert_eq!(
            apply_lifecycle_event(RunStatus::Running, &running),
            LifecycleTransition::Next(RunStatus::Running)
        );
        let removing = EventBody::RunRemoving(transition_props());
        // Repeated removal is an idempotent no-op like every repeat.
        assert_eq!(
            apply_lifecycle_event(RunStatus::Removing, &removing),
            LifecycleTransition::Next(RunStatus::Removing)
        );
    }
}
