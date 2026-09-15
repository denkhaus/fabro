//! Fork-only resume route (fabro-7627, salvaged from run
//! 01M2J1BA2JC4S6SA10YEJHHMVP 2026-09-15). Serves POST /runs/{id}/resume:
//! one-click resume of a terminal failed run via the engine's
//! `resume_from_failure` auto-rewind (fork module in fabro-workflow).
//!
//! Fork-file policy: this file exists only on our fork; upstream merges
//! cannot conflict it away. The seam is the single `merge` line in
//! `handler/lifecycle.rs::routes()`; presence is pinned by the OpenAPI
//! path test in fabro-workflow's `fork_seam_tests.rs` and the conflict
//! conformance test in `tests/it/api/fork_resume.rs`.

use std::sync::Arc;

use super::super::{
    AppState, IntoResponse, RequireRunManagementTarget, Response, State, StatusCode, operations,
    post, reject_if_archived,
};
use super::lifecycle::{queue_run_start, run_response, workflow_operation_error_response};

pub(in crate::server) fn routes() -> axum::Router<Arc<AppState>> {
    axum::Router::new().route("/runs/{id}/resume", post(resume_run))
}

async fn resume_run(
    RequireRunManagementTarget(id, actor): RequireRunManagementTarget,
    State(state): State<Arc<AppState>>,
) -> Response {
    if let Some(response) = reject_if_archived(state.as_ref(), &id).await {
        return response;
    }
    let input = operations::ResumeFailureInput { run_id: id };
    let outcome = Box::pin(operations::resume_from_failure(
        &state.stores.runs,
        &input,
        Some(actor.clone()),
    ))
    .await;
    match outcome {
        Ok(outcome) => {
            let new_run_id = outcome.new_run_id();
            if let Err(err) = queue_run_start(state.as_ref(), new_run_id, false, actor).await {
                return err.into_response();
            }
            let status = match outcome {
                operations::RewindOutcome::Full { .. } => StatusCode::CREATED,
                operations::RewindOutcome::Partial { .. } => StatusCode::MULTI_STATUS,
            };
            run_response(state.as_ref(), new_run_id, status).await
        }
        Err(err) => workflow_operation_error_response(err),
    }
}
