//! The orchestration doors (ADR-0039 stage D, denkhaus/work#1107): the
//! verdict write door, the adjudication queue read, and the run
//! observations read — the three surfaces the adjudicator's fast path
//! and agent pass share with their CLI twins (`fabro run mark`,
//! `fabro run queue`, `fabro observation list`).
//!
//! The write door is FOREIGN by design: the caller's own run (from the
//! worker principal) is the WRITER, the path's run is the TARGET — the
//! adjudicator and the merger write verdicts about OTHER runs. Which
//! workflow may write which field is a hardcoded pair (the stage-D
//! grill's carrier decision): `adjudicator` runs own `merge`, `merger`
//! runs own `closable`, and the door refuses every crossing write —
//! one writer per field (ADR-0039 decision 6), structural, not
//! conventional.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use fabro_api::types::{
    RunObservation, RunObservationList, RunOrchestrationMarkRequest, RunOrchestrationMarkResponse,
};
use fabro_store::platform_records::{
    ObservationRecordedRecord, PlatformRecord, PlatformRecordKind, RunOrchestrationRecord,
};
use fabro_types::{OrchestrationField, Principal, RunId};
use tracing::info;

use super::super::{AppState, ListResponse, run_records};
use crate::error::ApiError;
use crate::principal_middleware::RequiredRunManagementActor;

/// The workflow allowlist (ADR-0039 stage D grill, hardcoded carrier
/// decision): the only workflow slugs whose runs may write orchestration
/// verdicts, each bound to exactly one field. The names are stable,
/// line-owned contracts — a change is a deliberate code edit, not a
/// settings knob.
const MERGE_WRITER_WORKFLOW: &str = "adjudicator";
const CLOSABLE_WRITER_WORKFLOW: &str = "merger";

/// How many runs one queue read returns at most. The queue is the
/// adjudicator's worklist, not an archive walk: a gear that needs more
/// than this per pass has a backlog problem a bigger page will not fix.
const QUEUE_PAGE_LIMIT: u32 = 200;

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/runs/queue", get(list_queue))
        .route("/runs/{id}/orchestration", post(mark_orchestration))
        .route("/runs/{id}/observations", get(list_observations))
}

#[derive(Debug, serde::Deserialize)]
struct QueueQuery {
    field: Option<OrchestrationField>,
}

/// The adjudication queue: terminal runs minus the runs this field has
/// a verdict for (the store's anti-join, `adjudication_queue`).
async fn list_queue(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Query(query): Query<QueueQuery>,
) -> Response {
    let field = query.field.unwrap_or(OrchestrationField::Merge);
    let runs = match state
        .stores
        .run_summaries
        .adjudication_queue(field, QUEUE_PAGE_LIMIT, Utc::now())
        .await
    {
        Ok(runs) => runs,
        Err(err) => {
            return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                .into_response();
        }
    };
    let total = runs.len() as u64;
    let data = state.decorate_run_summaries(runs).await;
    Json(ListResponse::paginated(data, false, total)).into_response()
}

/// The verdict write door. Authorization is the identity chain of the
/// stage-D grill: the worker token names the WRITER run, the writer's
/// run summary names its workflow, the hardcoded pair binds the
/// workflow to the field it may write. The target run is the path.
async fn mark_orchestration(
    RequiredRunManagementActor(principal): RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(target): Path<RunId>,
    Json(request): Json<RunOrchestrationMarkRequest>,
) -> Result<Response, ApiError> {
    let Principal::Worker { run_id: writer } = principal else {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "Only workflow runs write orchestration verdicts — the adjudicator and the merger \
             gears. Users read verdicts through the run view.",
            "orchestration_not_allowed",
        ));
    };
    if writer == target {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "A run cannot mark itself — the adjudicator and the merger write verdicts about \
             OTHER runs (ADR-0039 decision 1: a run never judges itself).",
            "orchestration_self_mark",
        ));
    }
    let Some(owned_field) = owned_field_for_writer(&state, &writer).await? else {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            format!(
                "This run's workflow is not on the orchestration allowlist — only the \
                 {MERGE_WRITER_WORKFLOW} and {CLOSABLE_WRITER_WORKFLOW} workflows write \
                 verdicts."
            ),
            "orchestration_not_allowed",
        ));
    };
    if request.field != owned_field {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            format!(
                "One writer per field (ADR-0039 decision 6): this run's workflow writes \
                 `{owned_field}`, not `{}`.",
                request.field
            ),
            "orchestration_field_not_owned",
        ));
    }

    // The target must exist; `require_projection` settles the projector
    // so the response reflects the verdict this call writes.
    run_records::require_projection(&state, target).await?;

    let record = PlatformRecord::RunOrchestration(RunOrchestrationRecord {
        field:   request.field,
        verdict: request.verdict,
        by_run:  Some(writer),
    });
    let stored = run_records::append(&state, target, record)
        .await
        .map_err(|err| {
            tracing::error!(
                error = ?err,
                target_run_id = %target,
                "orchestration verdict append failed"
            );
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Recording the orchestration verdict failed.",
            )
        })?;
    info!(
        target_run_id = %target,
        writer_run_id = %writer,
        field = %request.field,
        verdict = %request.verdict,
        seq = stored.seq,
        "orchestration verdict recorded"
    );
    Ok(Json(RunOrchestrationMarkResponse {
        run_id:  target.to_string(),
        field:   request.field,
        verdict: request.verdict,
        by_run:  writer.to_string(),
    })
    .into_response())
}

/// The field this writer's workflow owns, `None` when the workflow is
/// not on the allowlist. An unreadable writer summary is an internal
/// error, not a silent refusal: the token named a run the store cannot
/// see.
async fn owned_field_for_writer(
    state: &AppState,
    writer: &RunId,
) -> Result<Option<OrchestrationField>, ApiError> {
    let summary = state
        .stores
        .run_summaries
        .get(writer, Utc::now())
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, writer_run_id = %writer, "writer run summary read failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Reading the writer run's summary failed.",
            )
        })?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::FORBIDDEN,
                "The writer run named by the worker token does not exist.",
            )
        })?;
    Ok(match summary.workflow.slug.as_deref() {
        Some(MERGE_WRITER_WORKFLOW) => Some(OrchestrationField::Merge),
        Some(CLOSABLE_WRITER_WORKFLOW) => Some(OrchestrationField::Closable),
        _ => None,
    })
}

/// A run's observation records in `seq` order: the facts its stages
/// recorded, with the category that routes adjudication (a painpoint
/// forces the agent pass).
async fn list_observations(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<RunId>,
) -> Result<Response, ApiError> {
    run_records::require_projection(&state, run_id).await?;
    let records = state
        .stores
        .run_summaries
        .platform_records()
        .read_kind(&run_id, PlatformRecordKind::ObservationRecorded)
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, run_id = %run_id, "run observations read failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Reading the run's observations failed.",
            )
        })?;
    let observations = records
        .iter()
        .filter_map(|stored| match &stored.record {
            PlatformRecord::ObservationRecorded(record) => {
                Some(wire_observation(stored.seq, stored.recorded_at, record))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    Ok(Json(RunObservationList { observations }).into_response())
}

/// One stored observation record as the API serves it. The category is
/// the canonical `fabro_types::ObservationCategory` (reused through
/// `with_replacement` — no parallel DTO, no bridge).
fn wire_observation(
    seq: u64,
    recorded_at_ms: u64,
    record: &ObservationRecordedRecord,
) -> RunObservation {
    RunObservation {
        seq:         seq.cast_signed(),
        category:    record.category,
        text:        record.text.clone(),
        execution:   i64::try_from(record.execution).ok(),
        firing:      i64::try_from(record.firing).ok(),
        recorded_at: chrono::DateTime::from_timestamp_millis(recorded_at_ms.cast_signed()),
    }
}
