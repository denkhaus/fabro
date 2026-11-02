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

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use fabro_api::types::{
    PaginationMeta, RunObservation, RunObservationList, RunOrchestrationMarkRequest,
    RunOrchestrationMarkResponse, RunQueueEntry, RunQueueEntryVerdict, RunQueueList,
};
use fabro_store::platform_records::{
    ObservationRecordedRecord, PlatformRecord, PlatformRecordKind, RunOrchestrationRecord,
};
use fabro_types::{OrchestrationField, OrchestrationVerdict, Principal, RunId};
use tracing::info;

use super::super::{AppState, run_records};
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
    field:    Option<OrchestrationField>,
    workflow: Option<String>,
}

/// The adjudication queue: terminal runs whose latest verdict on the
/// field is not settled (pending or parked), optionally narrowed to one
/// workflow's runs — the adjudicator's worklist filter. Each entry is
/// decorated with the work item of the run's latest claim (released
/// claims included): the claim ledger is the run → work-item edge
/// (ADR-0039 stage D part 2).
async fn list_queue(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Query(query): Query<QueueQuery>,
) -> Response {
    let field = query.field.unwrap_or(OrchestrationField::Merge);
    // The +1 probe: `has_more` must be honest — a worklist bigger than
    // one page must never read as complete (the fast path would sweep
    // only the first page and route "Queue empty" beside a full
    // backlog).
    let fetched = match state
        .stores
        .run_summaries
        .adjudication_queue(
            field,
            query.workflow.as_deref(),
            QUEUE_PAGE_LIMIT + 1,
            Utc::now(),
        )
        .await
    {
        Ok(runs) => runs,
        Err(err) => {
            return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                .into_response();
        }
    };
    let has_more = usize::try_from(QUEUE_PAGE_LIMIT).is_ok_and(|limit| fetched.len() > limit);
    let runs = if has_more {
        let limit = usize::try_from(QUEUE_PAGE_LIMIT).expect("a page limit that fits a usize");
        fetched[..limit].to_vec()
    } else {
        fetched
    };

    // The verdict map names the parked runs: the queue holds pending and
    // parked entries, and only the parked ones re-entered by escalation.
    // Read once, like the store's own fold.
    let verdicts = match state
        .stores
        .run_summaries
        .platform_records()
        .latest_verdicts_by_run(field)
        .await
    {
        Ok(verdicts) => verdicts
            .into_iter()
            .collect::<HashMap<RunId, OrchestrationVerdict>>(),
        Err(err) => {
            return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                .into_response();
        }
    };
    let run_ids = runs.iter().map(|run| run.id).collect::<Vec<_>>();
    // The decoration carries the claim's item AND project (work#1113):
    // queue consumers read scoped doors (the closure radius) with the
    // claim's own project, never a hardcoded one.
    let claim_by_run = match state.stores.work.latest_claims_of_runs(&run_ids).await {
        Ok(claims) => claims
            .into_iter()
            .map(|claim| (claim.run_id, (claim.item_key, claim.project_id)))
            .collect::<HashMap<RunId, (String, String)>>(),
        Err(err) => {
            return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                .into_response();
        }
    };

    let data = state.decorate_run_summaries(runs).await;
    let entries = data
        .into_iter()
        .map(|run| {
            let verdict = match verdicts.get(&run.id) {
                Some(OrchestrationVerdict::Parked) => RunQueueEntryVerdict::Parked,
                _ => RunQueueEntryVerdict::Pending,
            };
            let (work_item, project) = match claim_by_run.get(&run.id) {
                Some((item_key, project_id)) => (Some(item_key.clone()), Some(project_id.clone())),
                None => (None, None),
            };
            RunQueueEntry {
                work_item,
                project,
                run,
                verdict,
            }
        })
        .collect::<Vec<_>>();
    Json(RunQueueList {
        data: entries,
        meta: PaginationMeta {
            has_more,
            // The full count is not computed (the +1 probe carries the
            // paging truth; `total` stays absent rather than reporting
            // the page length as the worklist size).
            total: None,
        },
    })
    .into_response()
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
