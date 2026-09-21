//! Fork terminal taxonomy on Petri's projection fold (fabro-6655, W1-3).
//!
//! Upstream's conclusion only knows Completed / Cancelled / WorkflowError.
//! The fork extends the vocabulary without touching engine semantics:
//!
//! - `Succeeded { PublishBlocked }` (fabro-67e5): the run finished its graph
//!   green, but the pull-request publish failed. The remediation text tells the
//!   operator whether the run branch was pushed (work safe remotely) or not
//!   (work preserved in checkpoints).
//! - `Succeeded { Boundary }` and `Failed { Deadlock | SoftStop }` exit kinds
//!   (fabro-08b4, fabro-b907/ADR-0010) dock here when W3-2 maps the Attractor
//!   failure tiers onto this seam.
//!
//! Both call sites are one-line seams: `coordinator::conclude` asks for
//! the success reason, and the fold's `PullRequestFailed` handler
//! re-classifies an already-concluded success (records can arrive after
//! the conclusion).

use chrono::{DateTime, Utc};
use fabro_types::{
    FailureCategory, FailureDetail, FailureReason, RunFailure, RunStatus, SuccessReason,
};

/// Whether the projection's pull-request creation has failed (the publish
/// half of a completed run).
pub(crate) fn publish_creation_failed(creation: Option<&fabro_types::PullRequestCreation>) -> bool {
    creation.is_some_and(|creation| creation.error.is_some())
}

/// Build the publish-blocked failure detail for the conclusion (fabro-67e5
/// remediation text, adapted: Petri records carry the creation error).
pub(crate) fn publish_blocked_failure(creation_error: &str, at: DateTime<Utc>) -> RunFailure {
    let _ = at;
    RunFailure {
        reason: FailureReason::PublishFailed,
        detail: FailureDetail::new(
            format!(
                "Work done, publish blocked — the pull request could not be created: \
                 {creation_error}. Retry the run or open the pull request manually; the \
                 run branch and checkpoints preserve the work."
            ),
            FailureCategory::TransientInfra,
        ),
    }
}

/// Re-classify an already-concluded success when the publish failure record
/// arrives after the conclusion (the fold processes records in order, and
/// `pull_request.failed` can trail `ExecutionFinished`).
pub(crate) fn reclassify_publish_blocked(
    projection: &mut fabro_types::RunProjection,
    at: DateTime<Utc>,
) {
    let status = projection.status;
    if let RunStatus::Succeeded {
        reason: SuccessReason::Completed,
    } = status
    {
        projection.status = RunStatus::Succeeded {
            reason: SuccessReason::PublishBlocked,
        };
        let error = projection
            .pull_request_creation
            .as_ref()
            .and_then(|creation| creation.error.clone())
            .unwrap_or_else(|| "unknown error".to_string());
        if let Some(conclusion) = projection.conclusion.as_mut() {
            conclusion.failure = Some(publish_blocked_failure(&error, at));
        }
        apply_status_note(projection, at);
    }
}

/// Keep `status_updated_at` fresh on re-classification.
fn apply_status_note(projection: &mut fabro_types::RunProjection, at: DateTime<Utc>) {
    projection.status_updated_at = at;
}
