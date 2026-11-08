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
    ObservationWorkItemLink, PaginationMeta, RunMergeResponse, RunObservation, RunObservationList,
    RunOrchestrationMarkRequest, RunOrchestrationMarkResponse, RunQueueEntry, RunQueueEntryVerdict,
    RunQueueList,
};
use fabro_store::platform_records::{
    ObservationRecordedRecord, PlatformRecord, PlatformRecordKind, RunOrchestrationRecord,
};
use fabro_types::settings::run::MergeStrategy;
use fabro_types::{
    OrchestrationField, OrchestrationVerdict, Principal, PullRequestLink, PullRequestState, RunId,
};
use tracing::info;

use super::super::{AppState, run_records};
use crate::error::ApiError;
use crate::principal_middleware::RequiredRunManagementActor;

/// The workflow allowlist (ADR-0039 stage D grill, hardcoded carrier
/// decision): the only workflow slugs whose runs may write orchestration
/// verdicts, each bound to exactly one field. The names are stable,
/// line-owned contracts — a change is a deliberate code edit, not a
/// settings knob. Shared with the work-item close door (stage E): the
/// adjudicator is the only workflow that closes work items.
pub(super) const MERGE_WRITER_WORKFLOW: &str = "adjudicator";
pub(super) const CLOSABLE_WRITER_WORKFLOW: &str = "merger";

/// How many runs one queue read returns at most. The queue is the
/// adjudicator's worklist, not an archive walk: a gear that needs more
/// than this per pass has a backlog problem a bigger page will not fix.
const QUEUE_PAGE_LIMIT: u32 = 200;

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/runs/queue", get(list_queue))
        .route("/runs/{id}/orchestration", post(mark_orchestration))
        .route("/runs/{id}/merge", post(merge_run))
        .route("/runs/{id}/observations", get(list_observations))
}

#[derive(Debug, serde::Deserialize)]
struct QueueQuery {
    field:    Option<OrchestrationField>,
    workflow: Option<String>,
    /// Settled-verdict filter (stage E): flips the anti-join — the queue
    /// lists exactly the runs whose latest verdict on the field IS this
    /// value. The closing pass reads `closable = enabled`.
    verdict:  Option<OrchestrationVerdict>,
    /// The other-field requirement (work#1126): `<field>=<verdict>`
    /// restricts the anti-join to runs whose latest verdict on the
    /// NAMED OTHER field IS this verdict — the merger's worklist is
    /// `field=closable` + `requires=merge=enabled`.
    requires: Option<String>,
}

/// Parse the `requires` pair (`<field>=<verdict>`) with teaching
/// refusals: exactly one `=`, both halves known enums, the OTHER field
/// (never the queue's own), and never beside a `verdict` filter (the
/// requirement belongs to the unsettled anti-join only).
fn parse_queue_requirement(
    raw: &str,
    field: OrchestrationField,
    verdict_filter_present: bool,
) -> Result<Option<(OrchestrationField, OrchestrationVerdict)>, ApiError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let raw = trimmed;
    if verdict_filter_present {
        return Err(ApiError::with_code(
            StatusCode::BAD_REQUEST,
            "`requires` belongs to the unsettled anti-join — it cannot combine with `verdict` \
             (the settled read flips the join; the requirement has no meaning there)",
            "queue_requires_conflict",
        ));
    }
    let Some((raw_field, raw_verdict)) = raw.split_once('=') else {
        return Err(ApiError::with_code(
            StatusCode::BAD_REQUEST,
            format!(
                "`{raw}` is not a field=verdict pair — the requirement reads, for example, \
                 merge=enabled"
            ),
            "queue_requires_malformed",
        ));
    };
    let other_field: OrchestrationField = serde_json::from_value(serde_json::Value::String(
        raw_field.to_string(),
    ))
    .map_err(|_| {
        ApiError::with_code(
            StatusCode::BAD_REQUEST,
            format!(
                "`{raw_field}` is not an orchestration field — the fields are merge and closable"
            ),
            "queue_requires_unknown_field",
        )
    })?;
    if other_field == field {
        return Err(ApiError::with_code(
            StatusCode::BAD_REQUEST,
            format!(
                "the requirement must name the OTHER field — `{raw_field}` is the queue's own \
                 field (a run settled on it has already left the anti-join)"
            ),
            "queue_requires_same_field",
        ));
    }
    let verdict: OrchestrationVerdict = serde_json::from_value(serde_json::Value::String(
        raw_verdict.to_string(),
    ))
    .map_err(|_| {
        ApiError::with_code(
            StatusCode::BAD_REQUEST,
            format!(
                "`{raw_verdict}` is not a verdict — the requirement accepts enabled or \
                         refused (pending is absence, parked re-enters the queue by itself)"
            ),
            "queue_requires_unknown_verdict",
        )
    })?;
    if !matches!(
        verdict,
        OrchestrationVerdict::Enabled | OrchestrationVerdict::Refused
    ) {
        return Err(ApiError::with_code(
            StatusCode::BAD_REQUEST,
            format!(
                "`{raw_verdict}` is not a settled verdict — the requirement accepts enabled or \
                 refused (pending is absence, parked re-enters the queue by itself)"
            ),
            "queue_requires_not_settled",
        ));
    }
    Ok(Some((other_field, verdict)))
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
    let requirement = match &query.requires {
        Some(raw) => match parse_queue_requirement(raw, field, query.verdict.is_some()) {
            Ok(requirement) => requirement,
            Err(error) => return error.into_response(),
        },
        None => None,
    };
    // The +1 probe: `has_more` must be honest — a worklist bigger than
    // one page must never read as complete (the fast path would sweep
    // only the first page and route "Queue empty" beside a full
    // backlog).
    let fetched = match query.verdict {
        // The settled-verdict read (stage E): the closing pass's
        // worklist — exactly the runs settled on the named verdict.
        Some(verdict) => {
            state
                .stores
                .run_summaries
                .verdict_queue(
                    field,
                    verdict,
                    query.workflow.as_deref(),
                    QUEUE_PAGE_LIMIT + 1,
                    Utc::now(),
                )
                .await
        }
        None => {
            state
                .stores
                .run_summaries
                .adjudication_queue(
                    field,
                    requirement,
                    query.workflow.as_deref(),
                    QUEUE_PAGE_LIMIT + 1,
                    Utc::now(),
                )
                .await
        }
    };
    let fetched = match fetched {
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
                Some(OrchestrationVerdict::Enabled) => RunQueueEntryVerdict::Enabled,
                Some(OrchestrationVerdict::Refused) => RunQueueEntryVerdict::Refused,
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

/// The merger's merge door (ADR-0039 stage E): merge the TARGET run's
/// pull request server-side — the technical integration act, the
/// supervisor's merge machinery moved into a door. The authorization
/// chain mirrors the verdict door: the worker token names the WRITER,
/// the writer's workflow must be the merger (the hardcoded allowlist),
/// and the TARGET must carry a pull request whose latest `merge`
/// verdict is `enabled` — the adjudicator's content verdict; the merger
/// never judges content. This door writes NO verdict (one door, one
/// write): the merger marks `closable` through the verdict door after
/// the merge (and its smoke) settles.
async fn merge_run(
    RequiredRunManagementActor(principal): RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(target): Path<RunId>,
) -> Result<Response, ApiError> {
    let Principal::Worker { run_id: writer } = principal else {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "Only workflow runs merge pull requests — the merger gear. Users merge through \
             GitHub.",
            "merge_not_allowed",
        ));
    };
    if writer == target {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "A run cannot merge itself — the merger integrates OTHER runs (ADR-0039 decision 1).",
            "merge_self",
        ));
    }
    let Some(owned_field) = owned_field_for_writer(&state, &writer).await? else {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            format!(
                "This run's workflow is not on the orchestration allowlist — only the \
                 {MERGE_WRITER_WORKFLOW} and {CLOSABLE_WRITER_WORKFLOW} workflows write \
                 verdicts or merge."
            ),
            "merge_not_allowed",
        ));
    };
    if owned_field != OrchestrationField::Closable {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            format!(
                "Only the {CLOSABLE_WRITER_WORKFLOW} merges pull requests — this run's workflow \
                 writes `{owned_field}` verdicts, it does not integrate."
            ),
            "merge_not_merger",
        ));
    }
    // The target must exist and settle its projection — the PR and the
    // verdict fold read off it.
    let projection = run_records::require_projection(&state, target).await?;
    let Some(record) = projection.pull_request.clone() else {
        return Err(ApiError::with_code(
            StatusCode::CONFLICT,
            "The run carries no pull request — there is nothing to merge. The adjudicator's \
             pass one refuses PR-less runs; the merger never sees one.",
            "merge_no_pull_request",
        ));
    };
    if state
        .stores
        .run_summaries
        .platform_records()
        .latest_verdict_of_run(OrchestrationField::Merge, &target)
        .await
        .map_err(|err| {
            tracing::error!(error = ?err, target_run_id = %target, "merge verdict read failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Reading the run's merge verdict failed.",
            )
        })?
        != Some(OrchestrationVerdict::Enabled)
    {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "The adjudicator has not enabled this run's merge — the merger integrates only \
             merge-enabled runs (ADR-0039 decision 6: the content verdict precedes the \
             technical merge).",
            "merge_not_enabled",
        ));
    }

    // The merge itself: server-side GitHub credentials (the supervisor's
    // machinery — squash, the line's merge shape).
    let creds = super::pull_requests::load_server_github_credentials(&state).await?;
    let github = super::pull_requests::server_github_context(&state, &creds)?;

    // Idempotence (work#1127, the grilled explicit state read): a
    // re-drive of an already-merged pull request answers 200
    // immediately — a crash between the merge and the closable mark
    // must not strand a merged run as a phantom conflict packet. The
    // probe FAILS OPEN: on a read error the merge call itself stays
    // the authority (GitHub answers 405/409 for an already-merged PR
    // only when the state read could not prove it).
    let already_merged =
        match fabro_github::get_pull_request(&github, &record.owner, &record.repo, record.number)
            .await
        {
            Ok(detail) => detail.merged,
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    target_run_id = %target,
                    pr_number = record.number,
                    "merge door: the idempotence state read failed — proceeding to the merge call"
                );
                false
            }
        };
    if already_merged {
        info!(
            target_run_id = %target,
            writer_run_id = %writer,
            pr_number = record.number,
            "run pull request already merged (merger door, idempotent re-drive)"
        );
        record_pull_request_merged(&state, target, &record).await;
        return Ok(Json(RunMergeResponse {
            run_id:       target.to_string(),
            merged:       true,
            pull_request: record.html_url(),
        })
        .into_response());
    }

    let merge = fabro_github::merge_pull_request(
        &github,
        &record.owner,
        &record.repo,
        record.number,
        MergeStrategy::Squash,
    )
    .await;
    match merge {
        Ok(()) => {
            info!(
                target_run_id = %target,
                writer_run_id = %writer,
                pr_number = record.number,
                "run pull request merged (merger door, squash)"
            );
            record_pull_request_merged(&state, target, &record).await;
            Ok(Json(RunMergeResponse {
                run_id:       target.to_string(),
                merged:       true,
                pull_request: record.html_url(),
            })
            .into_response())
        }
        Err(fabro_github::PullRequestApiError::Conflict { status, body, .. }) => {
            // The typed conflict signal (fabro #1117): the fast path
            // parks the run for the agent pass on exactly this code.
            Err(ApiError::with_code(
                StatusCode::CONFLICT,
                format!("The pull request cannot merge (status {status}): {body}"),
                "merge_conflict",
            ))
        }
        Err(error) => {
            tracing::error!(
                error = ?error,
                target_run_id = %target,
                pr_number = record.number,
                "run pull request merge failed"
            );
            Err(ApiError::with_code(
                StatusCode::BAD_GATEWAY,
                format!("The GitHub merge call failed: {error}"),
                "merge_failed",
            ))
        }
    }
}

/// Record the merge outcome on the run's stored pull request link
/// (denkhaus/work#1173): a `pull_request.merged` platform record the
/// projection folds onto `pull_request.state`, so the run list — and the
/// board's PR chip — carries the outcome. Idempotent: a link already
/// settled `merged` is not re-recorded. A record failure is logged, never
/// a failed response: the merge is a fact on GitHub either way, and the
/// merged-wait's observation is the second writer that recovers the
/// state.
async fn record_pull_request_merged(state: &AppState, target: RunId, record: &PullRequestLink) {
    if record.state == Some(PullRequestState::Merged) {
        return;
    }
    if let Err(err) = run_records::append(state, target, PlatformRecord::PullRequestMerged).await {
        tracing::error!(
            error = ?err,
            target_run_id = %target,
            pr_number = record.number,
            "recording the merged pull request state failed"
        );
    }
}

/// A run's observation records in `seq` order: the facts its stages
/// recorded, with the category that routes adjudication (a painpoint
/// forces the agent pass). Each record carries its stage attribution
/// (the projection's positioned fold — `StageProjection.observations`)
/// and its linked work items (the ADR-0038 link join), the two columns
/// the run detail view's observations table renders (denkhaus/work#1098).
async fn list_observations(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<RunId>,
) -> Result<Response, ApiError> {
    let projection = run_records::require_projection(&state, run_id).await?;
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
    // The positioned attribution: which stage each record folded onto.
    // A record the fold did not place (a positionless server-origin
    // record, e.g. the run-end hook's own friction notes) keeps no
    // stage and renders un-attributed.
    let stage_of_seq: HashMap<u64, String> = projection
        .iter_stages()
        .flat_map(|(stage_id, stage)| {
            stage
                .observations
                .iter()
                .map(|observation| (observation.seq, stage_id.to_string()))
        })
        .collect();
    // The link join (ADR-0038): the work items filed from or backfilled
    // onto each record. A store failure degrades to an empty column —
    // the observations themselves still list (the radius precedent).
    let mut links_of_seq: HashMap<u64, Vec<ObservationWorkItemLink>> = HashMap::new();
    match state.work_store().linked_items_for_run(&run_id).await {
        Ok(links) => {
            for link in links {
                links_of_seq
                    .entry(link.seq)
                    .or_default()
                    .push(ObservationWorkItemLink {
                        item_key:  link.item_key,
                        linked_at: Some(link.linked_at),
                    });
            }
        }
        Err(err) => {
            tracing::warn!(
                run_id = %run_id,
                error = %err,
                "observation link read failed; the linked-item column degrades to empty"
            );
        }
    }
    let observations = records
        .iter()
        .filter_map(|stored| match &stored.record {
            PlatformRecord::ObservationRecorded(record) => Some(wire_observation(
                stored.seq,
                stored.recorded_at,
                record,
                stage_of_seq.get(&stored.seq).cloned(),
                links_of_seq.get(&stored.seq).cloned().unwrap_or_default(),
            )),
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
    stage: Option<String>,
    linked_work_items: Vec<ObservationWorkItemLink>,
) -> RunObservation {
    RunObservation {
        seq: seq.cast_signed(),
        category: record.category,
        text: record.text.clone(),
        execution: i64::try_from(record.execution).ok(),
        firing: i64::try_from(record.firing).ok(),
        stage,
        linked_work_items,
        recorded_at: chrono::DateTime::from_timestamp_millis(recorded_at_ms.cast_signed()),
    }
}
