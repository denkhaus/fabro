//! Fork terminal taxonomy (extracted per ADR-0021 D7 in the v0.357 merge;
//! user directive 2026-09-15: fork features live in fork-only files).
//!
//! Publish-blocked / boundary / soft-stop classification for terminal run
//! events (fabro-67e5, fabro-08b4, fabro-b907): upstream's finalize only
//! knows Completed / PartialSuccess / WorkflowError. The functions here
//! extend the taxonomy; finalize.rs calls them through one-line seams.

use std::time::SystemTime;

use fabro_llm::LONG_RATE_LIMIT_WINDOW;
use fabro_llm::gateway::{RateLimitWindow, reset_window};
use fabro_types::{FailureReason, RunFailure, StageOutcome};

use crate::error::{Error, run_failure_from_error};
use crate::outcome::{FailureDetail, Outcome};

/// The failure reason a failed outcome's detail parks under (fabro-a3d8).
///
/// A provider usage-window reset hours away arrives only as message prose.
/// Count-based retries cannot bridge it, so the run parks as a soft stop —
/// infrastructure could not finish, the run stays resumable and the next
/// pass re-enters after the window reopens — instead of a hard
/// `workflow_error`. Failures without a long reset window keep today's
/// hard-error mapping.
pub(crate) fn soft_stop_failure_reason(failure: &FailureDetail) -> FailureReason {
    match reset_window(&failure.message, SystemTime::now()) {
        // A naive (offset-less) reset timestamp has an unknown true wait;
        // park rather than guess a duration (fabro-0607).
        Some(RateLimitWindow::UnknownEta) => FailureReason::SoftStop,
        Some(RateLimitWindow::Reopens(window)) if window > LONG_RATE_LIMIT_WINDOW => {
            FailureReason::SoftStop
        }
        _ => FailureReason::WorkflowError,
    }
}

/// Upgrade a terminal engine error parked at a boundary exit into a
/// success-shaped outcome (fabro-08b4).
///
/// `kind="boundary"` on the exit edge declares: the failure parked the
/// loop, it did not break it. The error becomes the attached failure
/// detail (same surface as publish-blocked), so conclusion and terminal
/// event agree on green plus why-it-parked. Every other exit kind passes
/// the outcome through untouched.
pub(crate) fn apply_boundary_upgrade(
    outcome: Result<Outcome, Error>,
    exit_kind: &str,
) -> (Result<Outcome, Error>, Option<RunFailure>) {
    match outcome {
        Err(error) if exit_kind == "boundary" => {
            let mut parked = Outcome::success();
            parked.failure = Some(error.to_failure_detail());
            (
                Ok(parked),
                Some(run_failure_from_error(&error, error.failure_reason())),
            )
        }
        other => (other, None),
    }
}

/// Downgrade a success-shaped terminal routed through a `kind="deadlock"`
/// or `kind="soft"` exit into a failed terminal (fabro-18a5).
///
/// The exit stage itself usually succeeds — the guard noticed the deadlock
/// and routed the graph to the exit node — so the outcome alone reads green.
/// That misclassification let publish run (a pull request opened for code
/// whose gates never passed) and blocked resume in the CLI and web UI,
/// because a succeeded run has nothing to resume. The downgrade restores the
/// intended semantics before both decisions: the run fails with
/// [`FailureReason::Deadlock`] (work preserved, a human decides) or
/// [`FailureReason::SoftStop`] (infrastructure could not finish, the next
/// run re-enters). Work preservation does not depend on the terminal publish
/// push: checkpoint pushes during execution already carried the run branch.
pub(crate) fn apply_soft_exit_downgrade(
    outcome: Result<Outcome, Error>,
    exit_kind: &str,
) -> (Result<Outcome, Error>, Option<RunFailure>) {
    let reason = match exit_kind {
        "deadlock" => Some(FailureReason::Deadlock),
        "soft" => Some(FailureReason::SoftStop),
        _ => None,
    };
    match (outcome, reason) {
        (Ok(outcome), Some(reason))
            if matches!(
                outcome.status,
                StageOutcome::Succeeded | StageOutcome::PartiallySucceeded
            ) =>
        {
            let message = if reason == FailureReason::Deadlock {
                "run parked at a deadlock exit (kind=\"deadlock\") — work is preserved in \
                 checkpoints and the pushed run branch; a human decides next"
            } else {
                "run parked at a soft exit (kind=\"soft\") — infrastructure could not finish; \
                 the next run re-enters autonomously"
            };
            let error = Error::engine(message);
            let failure = run_failure_from_error(&error, reason);
            (Err(error), Some(failure))
        }
        (outcome, _) => (outcome, None),
    }
}

/// Convert a publish error into the terminal failure detail for a
/// publish-blocked run (fabro-67e5).
///
/// The remediation differs by how far delivery got: a pushed branch means the
/// work is safely on the remote and only the pull request is missing, while a
/// failed push keeps the work in checkpoints and the sandbox.
pub(crate) fn publish_failure_from_error(error: &Error, pushed_branch: Option<&str>) -> RunFailure {
    let mut failure = run_failure_from_error(error, FailureReason::PublishFailed);
    let remediation = match pushed_branch {
        Some(branch) => format!(
            "Work done, publish blocked — the run branch '{branch}' was pushed; fix the \
             token's pull-requests scope or open the pull request manually."
        ),
        None => "Work done, publish blocked — the run branch was NOT pushed; the work is \
                 preserved in checkpoints, retry the run or push the checkpoint manually."
            .to_string(),
    };
    if !failure.detail.message.is_empty() {
        failure.detail.message.push(' ');
    }
    failure.detail.message.push_str(&remediation);
    failure
}
