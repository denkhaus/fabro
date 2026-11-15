//! A run's checkpoint timeline, and the runs made from it: fork, rewind and
//! retry.
//!
//! The timeline is the run's `checkpoint` platform records, labelled with
//! the stages the projector folded them onto. A fork resolves a target on
//! that timeline (`@ordinal`, a node, or `node@visit`; the latest checkpoint
//! by default), creates the new run's row (`fabro_workflow::operations`),
//! seeds its records, checkpoints, snapshots and run branch from the source
//! (`fabro_petri::fork`), and queues it in resume mode, so its worker
//! acquires a fresh workspace, restores the checkpoint's commit into it and
//! continues from the position. A rewind is a fork of a terminal run that
//! archives the source and records `run.superseded` on it; a retry is a
//! fork of a terminal run at its last checkpoint, with the failed stage run
//! again when the run failed on one.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use fabro_api::types as api;
use fabro_automation::AutomationId;
use fabro_config::Storage;
use fabro_petri::fork::{self as petri_fork, ForkError, ForkRequest};
use fabro_petri::petri::RunStore;
use fabro_petri::platform_records::SqlitePlatformRecords;
use fabro_petri::{SqliteRunStore, admission};
use fabro_store::platform_records::RunNoticeRecord;
use fabro_store::{PlatformRecordKind, RunProjection};
use fabro_types::diagnostic::Severity;
use fabro_types::notice::RunNoticeLevel;
use fabro_types::{FailureReason, Principal, RunId, RunTarget};
use fabro_util::error as error_util;
use fabro_workflow::Error as WorkflowError;
use fabro_workflow::operations::{self, ForkTarget, ResolvedForkTarget, RunTimeline};
use tracing::{error, warn};

use super::super::{
    ApiError, AppState, RequireRunManagementTarget, RequiredUser, parse_run_id_path, run_records,
};
use super::lifecycle::{ArchiveAction, queue_run, run_archive_operation, run_response};
use super::runs::run_provenance;
use crate::automation_materializer::{AutomationRunMaterializeInput, RunMaterializeError};
use crate::petri_check;
use crate::run_rebase::{self as run_rebase_ops, RunRebaseError};
use crate::server::run_publish::{find_snapshot_repository, snapshots_dir};

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/runs/{id}/timeline", get(run_timeline))
        .route("/runs/{id}/fork", post(fork_run))
        .route("/runs/{id}/rewind", post(rewind_run))
        .route("/runs/{id}/retry", post(retry_run))
        .route("/runs/{id}/retry-latest", post(retry_latest_run))
}

/// Which operation a fork is made for: what it requires of the source and
/// what it records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ForkKind {
    Fork,
    Rewind,
    Retry,
}

/// A fork made and queued.
struct ForkOutcome {
    source_run_id: RunId,
    new_run_id:    RunId,
    target:        ResolvedForkTarget,
    rerun_last:    bool,
}

async fn run_timeline(
    _auth: RequiredUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Response {
    let id = match parse_run_id_path(&id) {
        Ok(id) => id,
        Err(response) => return response,
    };
    let projection = match run_records::require_projection(state.as_ref(), id).await {
        Ok(projection) => projection,
        Err(err) => return err.into_response(),
    };
    match timeline(state.as_ref(), id).await {
        Ok(timeline) => Json(timeline_response(&timeline, &projection)).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn fork_run(
    RequireRunManagementTarget(id, actor): RequireRunManagementTarget,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(LineageForceQuery { force }): Query<LineageForceQuery>,
    body: Option<Json<api::ForkRequest>>,
) -> Response {
    let target = match parse_fork_target(body.and_then(|Json(body)| body.target)) {
        Ok(target) => target,
        Err(err) => return err.into_response(),
    };
    match fork_at(
        state.as_ref(),
        id,
        actor,
        &headers,
        ForkKind::Fork,
        target,
        force,
    )
    .await
    {
        Ok(outcome) => (
            StatusCode::OK,
            Json(api::ForkResponse {
                source_run_id:  outcome.source_run_id.to_string(),
                new_run_id:     outcome.new_run_id.to_string(),
                target:         outcome.target.response_target(),
                checkpoint_sha: outcome.target.checkpoint_sha.clone(),
                execution:      outcome.target.position.execution,
                firing:         outcome.target.position.firing,
                rerun_last:     outcome.rerun_last,
            }),
        )
            .into_response(),
        Err(err) => err.into_response(),
    }
}

/// Fork (rewind) a terminal run internally: the handler's core minus the
/// HTTP surface (fork, archive, superseded record). Returns the new run id
/// and whether the source archive succeeded (fabro-2e7b Option C: the
/// provider gate re-fires parked runs through this instead of a new run).
pub(crate) async fn rewind_run_internal(
    state: &AppState,
    id: RunId,
    actor: Principal,
) -> Result<(RunId, bool), String> {
    let outcome = fork_at(
        state,
        id,
        actor,
        &HeaderMap::new(),
        ForkKind::Rewind,
        None,
        false,
    )
    .await
    .map_err(|err| format!("rewind rejected: {}", err.detail()))?;
    let archived = run_archive_operation(state, &id, None, ArchiveAction::Archive)
        .await
        .map(|_| ())
        .is_ok();
    if archived {
        let record = operations::superseded_record(outcome.new_run_id, &outcome.target);
        if let Err(err) = run_records::append(state, id, record).await {
            error!(
                source_run_id = %id,
                new_run_id = %outcome.new_run_id,
                error = %err,
                "the rewound run was archived but its superseded record was not written"
            );
        }
    }
    Ok((outcome.new_run_id, archived))
}

async fn rewind_run(
    RequireRunManagementTarget(id, actor): RequireRunManagementTarget,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(LineageForceQuery { force }): Query<LineageForceQuery>,
    body: Option<Json<api::RewindRequest>>,
) -> Response {
    let target = match parse_fork_target(body.and_then(|Json(body)| body.target)) {
        Ok(target) => target,
        Err(err) => return err.into_response(),
    };
    let outcome = match fork_at(
        state.as_ref(),
        id,
        actor.clone(),
        &headers,
        ForkKind::Rewind,
        target,
        force,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(err) => return err.into_response(),
    };
    // The source is replaced: archived, and marked with what replaced it.
    // A failed archive leaves the new run in place and says so.
    let archived = run_archive_operation(state.as_ref(), &id, Some(actor), ArchiveAction::Archive)
        .await
        .map(|_| ());
    if archived.is_ok() {
        let record = operations::superseded_record(outcome.new_run_id, &outcome.target);
        if let Err(err) = run_records::append(state.as_ref(), id, record).await {
            error!(
                source_run_id = %id,
                new_run_id = %outcome.new_run_id,
                error = %err,
                "the rewound run was archived but its superseded record was not written"
            );
        }
    }
    let (status, archive_error) = match archived {
        Ok(()) => (StatusCode::OK, None),
        Err(err) => (StatusCode::MULTI_STATUS, Some(err.to_string())),
    };
    (
        status,
        Json(api::RewindResponse {
            source_run_id: outcome.source_run_id.to_string(),
            new_run_id: outcome.new_run_id.to_string(),
            target: outcome.target.response_target(),
            checkpoint_sha: outcome.target.checkpoint_sha.clone(),
            execution: outcome.target.position.execution,
            firing: outcome.target.position.firing,
            archived: archive_error.is_none(),
            archive_error,
        }),
    )
        .into_response()
}

async fn retry_run(
    RequireRunManagementTarget(id, actor): RequireRunManagementTarget,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(LineageForceQuery { force }): Query<LineageForceQuery>,
) -> Response {
    match fork_at(
        state.as_ref(),
        id,
        actor,
        &headers,
        ForkKind::Retry,
        None,
        force,
    )
    .await
    {
        Ok(outcome) => run_response(state.as_ref(), outcome.new_run_id, StatusCode::CREATED).await,
        Err(err) => err.into_response(),
    }
}

/// The item thread the successor inherits (work#1320): the item the
/// source run worked — its live claim when the run still holds one, or
/// the run's latest claim record when park or failure already released
/// it (the 2026-11-14 case: the parked run's claim was gone, the item
/// association recoverable only from the ledger's history).
struct ItemThread {
    project_id: String,
    item_key:   String,
    lane:       String,
}

async fn source_item_thread(state: &AppState, id: &RunId) -> Option<ItemThread> {
    let store = state.work_store();
    if let Ok(Some(claim)) = store.active_claim_for_run(id).await {
        return Some(ItemThread {
            project_id: claim.project_id,
            item_key:   claim.item_key,
            lane:       claim.lane,
        });
    }
    let latest = store.latest_claims_of_runs(&[*id]).await.ok()?;
    latest.into_iter().next().map(|claim| ItemThread {
        project_id: claim.project_id,
        item_key:   claim.item_key,
        lane:       claim.lane,
    })
}

/// Move the item thread to the successor: release the source's live
/// claim, then grant the successor the same item — the atomic handoff
/// that closes the snatch window between click and the successor's
/// first planner act (the grill decision, 2026-11-14). Best effort: a
/// terminal source's claim is bookkeeping, never a reason to fail the
/// re-drive; every miss degrades to the planner's lane fallback.
async fn transfer_item_thread(
    state: &AppState,
    source: &RunId,
    successor: &RunId,
    thread: &ItemThread,
) {
    let store = state.work_store();
    if let Ok(Some(active)) = store.active_claim_for_run(source).await {
        let reason = "retry-on-latest: the successor continues this item";
        match store.release(&active.claim_id, reason).await {
            Ok(released) => {
                tracing::info!(
                    claim_id = %released.claim_id,
                    item_key = %released.item_key,
                    successor_run_id = %successor,
                    "retry-on-latest released the source claim"
                );
            }
            Err(err) => {
                tracing::warn!(claim_id = %active.claim_id, error = %err, "retry-on-latest could not release the source claim");
            }
        }
    }
    match store
        .grant(&fabro_work::GrantRequest {
            project_id: thread.project_id.clone(),
            item_key:   thread.item_key.clone(),
            run_id:     *successor,
            lane:       thread.lane.clone(),
        })
        .await
    {
        Ok(fabro_work::GrantOutcome::Granted(claim)) => {
            tracing::info!(
                claim_id = %claim.claim_id,
                item_key = %claim.item_key,
                successor_run_id = %successor,
                "retry-on-latest item thread transferred"
            );
        }
        // This run already holds the item — nothing to move.
        Ok(fabro_work::GrantOutcome::AlreadyHeld(_)) => {}
        Ok(fabro_work::GrantOutcome::Conflict(claim)) => {
            tracing::warn!(
                item_key = %thread.item_key,
                held_by_run_id = %claim.run_id,
                successor_run_id = %successor,
                "retry-on-latest item transfer lost the race; the planner falls back to the lane"
            );
        }
        Err(err) => {
            tracing::warn!(
                item_key = %thread.item_key,
                error = %err,
                "retry-on-latest item grant failed; the planner falls back to the lane"
            );
        }
    }
}

/// The run's timeline: its checkpoint records, labelled through the
/// projector's fold state.
async fn timeline(state: &AppState, id: RunId) -> Result<RunTimeline, ApiError> {
    let checkpoints = state
        .stores
        .run_summaries
        .platform_records()
        .read_kind(&id, PlatformRecordKind::Checkpoint)
        .await
        .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let labels = petri_fork::stage_labels(&state.stores.run_summaries.pool(), id)
        .await
        .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(RunTimeline::build(&checkpoints, &labels))
}

fn timeline_response(
    timeline: &RunTimeline,
    projection: &RunProjection,
) -> api::RunTimelineResponse {
    api::RunTimelineResponse {
        entries:     timeline
            .entries
            .iter()
            .map(|entry| api::TimelineEntryResponse {
                ordinal:        u64::try_from(entry.ordinal).unwrap_or(u64::MAX),
                checkpoint_seq: entry.checkpoint_seq,
                execution:      entry.position.execution,
                firing:         entry.position.firing,
                attempt:        entry.position.attempt,
                stage:          entry.stage_id.clone(),
                node_name:      entry.node_name.clone(),
                visit:          entry.visit,
                workspace:      entry.workspace.clone(),
                run_commit_sha: entry.run_commit_sha.clone(),
                diff_summary:   entry.diff_summary.as_ref().map(|summary| api::DiffSummary {
                    files_changed: summary.files_changed,
                    additions:     summary.additions,
                    deletions:     summary.deletions,
                }),
            })
            .collect(),
        forked_from: projection.forked_from.as_ref().map(fork_origin_response),
    }
}

fn fork_origin_response(origin: &fabro_types::ForkOrigin) -> api::ForkOrigin {
    api::ForkOrigin {
        source_run_id: origin.source_run_id.to_string(),
        execution:     origin.execution,
        firing:        origin.firing,
        rerun_last:    origin.rerun_last,
    }
}

fn parse_fork_target(target: Option<String>) -> Result<Option<ForkTarget>, ApiError> {
    target
        .map(|target| {
            target
                .parse::<ForkTarget>()
                .map_err(workflow_operation_error)
        })
        .transpose()
}

/// The `?force` query override of the lineage endpoints (fabro-f93b):
/// fires a successor through a provider credential-readiness miss, keeping
/// the finding a warning — the same override the create path carries.
#[derive(Debug, Default, serde::Deserialize)]
struct LineageForceQuery {
    #[serde(default)]
    force: bool,
}

/// Make the fork: check the source, resolve the target, create the new
/// run's row, seed it from the source, and queue it in resume mode. The
/// successor's provider readiness is re-checked first (fabro-f93b).
async fn fork_at(
    state: &AppState,
    id: RunId,
    actor: Principal,
    headers: &HeaderMap,
    kind: ForkKind,
    target: Option<ForkTarget>,
    force: bool,
) -> Result<ForkOutcome, ApiError> {
    let source = run_records::require_projection(state, id).await?;
    match kind {
        ForkKind::Fork => operations::ensure_forkable(&source, &id),
        ForkKind::Rewind => operations::ensure_rewindable(&source, &id),
        ForkKind::Retry => operations::ensure_retryable(&source, &id),
    }
    .map_err(workflow_operation_error)?;
    refuse_unready_successor(state, &source, force).await?;
    let timeline = timeline(state, id).await?;
    let entry = match kind {
        ForkKind::Retry => timeline.latest(),
        ForkKind::Fork | ForkKind::Rewind => timeline.resolve_or_latest(target.as_ref()),
    }
    .map_err(workflow_operation_error)?;
    let resolved = ResolvedForkTarget::of(entry).map_err(workflow_operation_error)?;
    let rerun_last = kind == ForkKind::Retry && operations::reruns_last(source.status);

    // A position Petri would refuse is refused before the new run exists.
    let position = petri_fork::position(resolved.position.execution, resolved.position.firing);
    let store: Arc<dyn RunStore> = state
        .petri_projector
        .observe_store(Arc::new(SqliteRunStore::new(state.db_pool.clone())));
    petri_fork::check(store.as_ref(), id, position)
        .await
        .map_err(|err| fork_error(&err))?;

    let new_run_id = RunId::new();
    let storage = Storage::new(state.server_storage_dir());
    let source_run_dir = storage.run_scratch(&id).root().to_path_buf();
    let run_dir = storage.run_scratch(&new_run_id).root().to_path_buf();
    let provenance = (kind == ForkKind::Retry).then(|| run_provenance(headers, &actor));
    operations::persist_forked_run(state.store_ref().as_ref(), &operations::ForkedRunInput {
        source: &source,
        new_run_id,
        run_dir: run_dir.clone(),
        checkpoint_sha: resolved.checkpoint_sha.clone(),
        provenance,
        web_url: state.run_web_url(&new_run_id),
        retried_from: (kind == ForkKind::Retry).then_some(id),
    })
    .await
    .map_err(workflow_operation_error)?;

    let seeded = petri_fork::fork(ForkRequest {
        source: id,
        fork: new_run_id,
        source_run_dir: source_run_dir.join("petri"),
        fork_run_dir: run_dir.join("petri"),
        store,
        records: Arc::new(SqlitePlatformRecords::new(Arc::clone(
            &state.stores.run_summaries,
        ))),
        position,
        rerun_last,
        settings: source.spec.settings.run.clone(),
    })
    .await;
    if let Err(err) = seeded {
        // The new run's row exists and holds nothing to continue from: it
        // is reported failed with the reason, rather than left submitted.
        let message = error_util::collect_chain(&err).join(": ");
        warn!(source_run_id = %id, new_run_id = %new_run_id, error = %message, "the fork could not be seeded");
        if let Err(record_err) = run_records::lifecycle(
            state,
            new_run_id,
            run_records::failed(FailureReason::WorkflowError, message.clone()),
        )
        .await
        {
            error!(new_run_id = %new_run_id, error = %record_err, "the fork's failure was not recorded");
        }
        return Err(fork_error(&err));
    }

    // The fork continues as a run left in flight does: queued in resume
    // mode, on the projection its seeded records folded to.
    let projection = run_records::require_projection(state, new_run_id).await?;
    queue_run(state, new_run_id, &projection, true, actor).await?;
    Ok(ForkOutcome {
        source_run_id: id,
        new_run_id,
        target: resolved,
        rerun_last,
    })
}

/// The credential readiness of a successor run (fabro-f93b): the source's
/// stored admission names the model stages the successor will run, and the
/// current ready set judges them — a provider that lost its stored
/// credential since the source was admitted refuses the fork, rewind or
/// retry with the create path's rule (fabro-b46e), naming provider, model
/// and the expected secret, before any successor record exists. `force`
/// keeps the findings as warnings. An admission that cannot be loaded
/// fails closed: the successor would fail its start the same way.
async fn refuse_unready_successor(
    state: &AppState,
    source: &RunProjection,
    force: bool,
) -> Result<(), ApiError> {
    let ready = state.ready_llm_provider_ids().await;
    let graphs = admission::load(&state.store_ref().blobs(), &source.spec.admission)
        .await
        .map_err(|err| {
            let chain = error_util::collect_chain(&err).join(": ");
            error!(
                source_run_id = %source.spec.run_id,
                error = %chain,
                "a successor's readiness check could not load the source admission"
            );
            ApiError::with_code(
                StatusCode::INTERNAL_SERVER_ERROR,
                "the source run's admitted graphs could not be loaded",
                "admission_unreadable",
            )
        })?;
    let mut diagnostics = Vec::new();
    if ready.is_empty() && graphs.needs_model() {
        diagnostics.push(petri_check::no_ready_provider_diagnostic());
    }
    let requirements = graphs.model_requirements();
    diagnostics.extend(petri_check::model_readiness_diagnostics(
        &petri_check::ModelReadiness {
            catalog: state.catalog().as_ref(),
            ready: &ready,
            force,
        },
        &requirements,
    ));
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .collect::<Vec<_>>();
    if errors.is_empty() {
        for warning in &diagnostics {
            warn!(
                source_run_id = %source.spec.run_id,
                code = %warning.rule,
                message = %warning.message,
                "forced successor fires through a readiness finding (fabro-f93b)"
            );
        }
        return Ok(());
    }
    let listed = errors
        .iter()
        .map(|diagnostic| format!("{}: {}", diagnostic.rule, diagnostic.message))
        .collect::<Vec<_>>()
        .join("; ");
    // One code per refusal reason, the b869 precedent: the zero-ready arm
    // is its own rule and says so, the per-provider arm is the b46e rule.
    let code = if errors
        .iter()
        .any(|diagnostic| diagnostic.rule == petri_check::NO_READY_PROVIDER_RULE)
    {
        "no_ready_provider"
    } else {
        "provider_not_ready"
    };
    Err(ApiError::with_code(
        StatusCode::UNPROCESSABLE_ENTITY,
        format!(
            "the new run would die in its model stages: {listed} — store the credential or \
             pass --force (?force=true) to fire anyway"
        ),
        code,
    ))
}

/// A refused position is the caller's mistake; anything else is the
/// server's.
fn fork_error(err: &ForkError) -> ApiError {
    let message = error_util::collect_chain(err).join(": ");
    let status = match err {
        ForkError::Refused(_) => StatusCode::BAD_REQUEST,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    ApiError::new(status, message)
}

fn workflow_operation_error(err: WorkflowError) -> ApiError {
    match err {
        WorkflowError::Parse(message) | WorkflowError::Validation(message) => {
            ApiError::bad_request(message)
        }
        WorkflowError::Precondition(message) => ApiError::new(StatusCode::CONFLICT, message),
        WorkflowError::RunNotFound(_) => ApiError::not_found("Run not found."),
        err => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
    }
}

/// Retry-on-latest resolution (work#1320 Part B): MATERIALIZE, never
/// resolve — the successor creation compiles the workflow from the
/// target branch's CURRENT tip, the same path the automation
/// materializer takes at fire time, and registers that content as the
/// version to re-admit. The materialized target carries the
/// checked-out tip sha: the successor's checkout is the tip, never the
/// source's pinned starting commit. An explicit workflow_version_id in
/// the request keeps bypassing this (the caller pins what to re-drive).
///
/// The source's workflow slug is the workflow selector: an
/// extensionless selector names `.fabro/workflows/<slug>/workflow.toml`
/// inside the tip checkout, the same resolution a gear's fire performs.
async fn retry_latest_materialize(
    state: &AppState,
    source: &RunProjection,
    target: RunTarget,
    successor: RunId,
) -> Result<(fabro_types::WorkflowVersionId, RunTarget), ApiError> {
    let Some(slug) = source.spec.workflow_slug.as_deref() else {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "the source run names no workflow slug; name a workflow_version_id explicitly",
        ));
    };
    // The branch selector: the source's pinned sha must not pin the
    // materialization — the tip's content is what registers.
    let RunTarget::Git(mut tip) = target else {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "retry-on-latest needs a git-backed source run (a Git target)",
        ));
    };
    tip.sha = None;
    let materialized = state
        .materialize_automation_run(AutomationRunMaterializeInput {
            // The id names the materialization's temp directory only; a
            // retry-on-latest is not an automation.
            automation_id:   AutomationId::new("retry-on-latest")
                .expect("'retry-on-latest' is a valid automation id by grammar"),
            target:          tip,
            workflow_source: None,
            workflow:        slug.to_string(),
            run_id:          successor,
            temp_root:       state.automation_temp_root(),
        })
        .await
        .map_err(|err| {
            let status = match &err {
                // State conflicts the caller can see and act on: the
                // workflow is gone from the tip, or the source's
                // coordinate stopped validating.
                RunMaterializeError::WorkflowNotFound { .. }
                | RunMaterializeError::InvalidTarget { .. }
                | RunMaterializeError::InvalidWorkflowSource { .. } => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            ApiError::new(status, error_util::collect_chain(&err).join(": "))
        })?;
    Ok((
        materialized.workflow_version_id,
        RunTarget::Git(materialized.target),
    ))
}

/// The newest checkpoint commit's sha of a run: the tip its snapshot
/// repository provably holds (a failed run never pushes its branch —
/// work#1320). `None` when the run recorded no checkpoint with a sha.
async fn latest_checkpoint_sha(state: &AppState, id: &RunId) -> Option<String> {
    let records = state
        .stores
        .run_summaries
        .platform_records()
        .read_kind(id, PlatformRecordKind::Checkpoint)
        .await
        .ok()?;
    records
        .iter()
        .rev()
        .find_map(|stored| match &stored.record {
            fabro_store::PlatformRecord::Checkpoint(record) => record.git_commit_sha.clone(),
            _ => None,
        })
}

/// Report the successor failed with the picture, the same way a fork
/// that could not be seeded is reported: the row exists, the reason is
/// its evidence.
async fn fail_retry_latest_successor(state: &AppState, new_run_id: RunId, message: String) {
    warn!(new_run_id = %new_run_id, error = %message, "the retry-on-latest successor could not be driven");
    if let Err(record_err) = run_records::lifecycle(
        state,
        new_run_id,
        run_records::failed(FailureReason::WorkflowError, message.clone()),
    )
    .await
    {
        error!(new_run_id = %new_run_id, error = %record_err, "the successor's failure was not recorded");
    }
}

/// Retry-on-latest (work#1320): re-drive a terminal run on current
/// bases — the workflow MATERIALIZED from the target branch's tip
/// (registered fresh, work#1320 Part B), the successor's worktree
/// prepared at the materialized tip with the source run branch's
/// checkpoint commits cherry-picked on top. The successor starts at
/// the graph's start (its earlier stages re-run on the carried files);
/// graph incompatibilities and carry conflicts fail it with the
/// picture. An explicit workflow_version_id skips the materialization
/// and re-admits exactly that version on the source's target.
async fn retry_latest_run(
    RequireRunManagementTarget(id, actor): RequireRunManagementTarget,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Option<Json<api::RetryLatestRequest>>,
) -> Response {
    let explicit_version = match body.and_then(|Json(body)| body.workflow_version_id) {
        // The typed branch: the WorkflowVersionId schema pattern already
        // validated it during deserialization. The untyped nullable
        // branch: a pattern-valid string parses; an invalid one is
        // rejected — an explicit id is never silently resolved to the
        // current one.
        None | Some(api::RetryLatestRequestWorkflowVersionId::Variant1(None)) => None,
        Some(api::RetryLatestRequestWorkflowVersionId::Variant0(id)) => Some(id),
        Some(api::RetryLatestRequestWorkflowVersionId::Variant1(Some(raw))) => match raw.parse() {
            Ok(parsed) => Some(parsed),
            Err(_) => {
                return ApiError::bad_request(
                    "workflow_version_id is not a valid workflow version id (64 hex chars)",
                )
                .into_response();
            }
        },
    };
    let source = match run_records::require_projection(state.as_ref(), id).await {
        Ok(source) => source,
        Err(err) => return err.into_response(),
    };
    if let Err(err) = operations::ensure_retryable(&source, &id) {
        return workflow_operation_error(err).into_response();
    }
    let (Some(target), Some(git)) = (source.spec.target.clone(), source.spec.git.clone()) else {
        return ApiError::new(
            StatusCode::CONFLICT,
            "retry-on-latest needs a git-backed source run (target and git context)",
        )
        .into_response();
    };
    let Some(source_base) = git.sha.clone() else {
        return ApiError::new(
            StatusCode::CONFLICT,
            "the source run names no starting commit; its checkpoint work cannot be carried",
        )
        .into_response();
    };

    // The successor's id mints before resolution: the materialization
    // names its temp directory by it.
    let new_run_id = RunId::new();
    // Resolution (work#1320 Part B): materialize from the branch tip
    // unless the request pins a version — the explicit fast path keeps
    // the source's target and bypasses materialization.
    let (version, target) = match explicit_version {
        Some(version) => (version, target),
        None => match retry_latest_materialize(state.as_ref(), &source, target, new_run_id).await {
            Ok(resolved) => resolved,
            Err(err) => return err.into_response(),
        },
    };
    if Some(&version) == source.spec.workflow_version_id.as_ref() {
        // Names both sides: the refusal self-diagnoses (work#1320's door
        // ergonomics — the 2026-11-14 click needed code archaeology to
        // learn the platform deploy had not moved the version).
        let source_version = source
            .spec
            .workflow_version_id
            .as_ref()
            .map_or_else(|| "none".to_string(), ToString::to_string);
        let head = if explicit_version.is_some() {
            "the requested workflow version"
        } else {
            "the workflow at the branch tip"
        };
        return ApiError::new(
            StatusCode::CONFLICT,
            format!(
                "{head} ({version}) is the one the source already ran \
                 ({source_version}); a plain retry replays it",
            ),
        )
        .into_response();
    }

    // The successor: the create path compiles the resolved version with
    // the source's posture (target, environment, model, labels). The
    // materialized arm's target names the checked-out tip; the explicit
    // arm keeps the source's target as the caller pinned the version.
    // The item thread (work#1320): the goal names the source's item so
    // the successor's planner takes the itemKey fast path instead of a
    // fresh lane grant; the claim transfer below makes the handoff
    // atomic.
    let item_thread = source_item_thread(state.as_ref(), &id).await;
    let goal = match (&item_thread, !source.spec.graph.goal.is_empty()) {
        (Some(thread), true) => Some(format!(
            "{} — this re-drive continues {}",
            source.spec.graph.goal, thread.item_key
        )),
        (Some(thread), false) => Some(format!("This re-drive continues {}", thread.item_key)),
        (None, has_goal) => has_goal.then(|| source.spec.graph.goal.clone()),
    };
    let environment_id = source.spec.settings.run.environment.id.clone();
    let created = Box::pin(super::runs::create_run_from_intent(
        Arc::clone(&state),
        super::runs::CreateRunFromIntentRequest {
            intent:          fabro_types::RunIntent {
                workflow_version_id: version,
                target,
                args: fabro_types::RunIntentArgs {
                    model:            source.spec.settings.run.model.name.clone(),
                    provider:         None,
                    inputs:           std::collections::HashMap::default(),
                    preserve_sandbox: None,
                    dry_run:          None,
                    auto_approve:     None,
                    labels:           source.spec.labels.clone(),
                },
                environment_id: (!environment_id.is_empty()).then_some(environment_id),
                parent_id: None,
                title: Some(source.title().into_owned()),
                goal,
                force: None,
            },
            explicit_run_id: Some(new_run_id),
            actor:           actor.clone(),
            headers:         headers.clone(),
            automation:      None,
        },
    ))
    .await;
    if !created.status().is_success() {
        return created;
    }

    // Graph compatibility: the seeded-free successor runs the NEW
    // graph; every node id the SOURCE graph named must still exist.
    let projection = match run_records::require_projection(state.as_ref(), new_run_id).await {
        Ok(projection) => projection,
        Err(err) => return err.into_response(),
    };
    match operations::graph_compat(&source.spec.graph, &projection.spec.graph) {
        operations::RetryLatestCompat::Compatible => {}
        operations::RetryLatestCompat::NodesMissing { missing } => {
            let message = format!(
                "retry-on-latest: the new workflow version dropped node ids the source run used \
                 ({missing:?}); re-drive needs a graph that keeps them"
            );
            fail_retry_latest_successor(state.as_ref(), new_run_id, message.clone()).await;
            return ApiError::new(StatusCode::CONFLICT, message).into_response();
        }
    }

    // Carry the checkpoint work: the source run's commits since its
    // base, cherry-picked onto the prepared worktree. The SOURCE's
    // snapshot repository comes first — a failed run never pushes its
    // branch to the origin (work#1320); its work lives only in the
    // snapshot, and the newest checkpoint record's sha names the tip.
    // The origin stays the fallback (published branches of successful
    // runs, or a reaped snapshot).
    let worktree = Storage::new(state.server_storage_dir())
        .run_scratch(&new_run_id)
        .worktree_dir();
    let source_branch = format!("fabro/run/{id}");
    let carry_result = if let Some(tip) = latest_checkpoint_sha(state.as_ref(), &id).await {
        let snapshots = snapshots_dir(state.as_ref(), &id);
        match find_snapshot_repository(&snapshots, &tip).await {
            Some(repository) => {
                run_rebase_ops::carry_checkpoint_work_from_snapshot(
                    &worktree,
                    &repository,
                    &tip,
                    &source_base,
                )
                .await
            }
            // No snapshot holds the tip (reaped, or a foreign checkout):
            // the origin fallback — a branch that was never pushed
            // carries nothing, the re-drive starts fresh.
            None => {
                run_rebase_ops::carry_checkpoint_work(
                    &worktree,
                    &git.origin_url,
                    &source_branch,
                    &source_base,
                )
                .await
            }
        }
    } else {
        run_rebase_ops::carry_checkpoint_work(
            &worktree,
            &git.origin_url,
            &source_branch,
            &source_base,
        )
        .await
    };
    let carried = match carry_result {
        Ok(Ok(carried)) => carried,
        Ok(Err(RunRebaseError::NoSourceBranch { .. })) => run_rebase_ops::Rebased {
            commits:     0,
            source_base: source_base.clone(),
            source_tip:  source_base.clone(),
            touched:     Vec::new(),
        },
        Ok(Err(RunRebaseError::Conflict { files })) => {
            let message = format!(
                "retry-on-latest: the checkpoint work conflicts on {files:?}; resolve the \
                 carry by hand (the source branch is {source_branch})"
            );
            fail_retry_latest_successor(state.as_ref(), new_run_id, message.clone()).await;
            return ApiError::new(StatusCode::CONFLICT, message).into_response();
        }
        Ok(Err(other)) | Err(other) => {
            let message = error_util::collect_chain(&other).join(": ");
            fail_retry_latest_successor(state.as_ref(), new_run_id, message.clone()).await;
            return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, message).into_response();
        }
    };

    // The re-drive's provenance on the successor's stream: what was
    // carried, from where, onto what.
    let notice = fabro_store::PlatformRecord::RunNotice(RunNoticeRecord {
        level:   RunNoticeLevel::Info,
        code:    "retry_on_latest".to_string(),
        message: format!(
            "re-drive of run {id}: workflow version {version}, checkpoint work carried: \
             {} commit(s) from {source_branch} (base {source_base}, tip {})",
            carried.commits, carried.source_tip
        ),
    });
    if let Err(err) = run_records::append(state.as_ref(), new_run_id, notice).await {
        warn!(new_run_id = %new_run_id, error = %err, "the re-drive notice was not recorded");
    }

    // The claim handoff lands before the run starts: the successor
    // holds the item when its planner first fires.
    if let Some(thread) = &item_thread {
        transfer_item_thread(state.as_ref(), &id, &new_run_id, thread).await;
    }

    if let Err(err) = queue_run(state.as_ref(), new_run_id, &projection, false, actor).await {
        return err.into_response();
    }
    (
        StatusCode::CREATED,
        Json(api::RetryLatestResponse {
            source_run_id:       id.to_string(),
            new_run_id:          new_run_id.to_string(),
            workflow_version_id: version.to_string(),
            carried_commits:     carried.commits as u64,
            touched_files:       carried.touched.clone(),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod retry_latest_item_thread_tests {
    use std::collections::BTreeMap;

    use fabro_types::RunId;
    use fabro_work::{ClaimState, GrantOutcome, GrantRequest, WorkProject};

    use super::{source_item_thread, transfer_item_thread};

    fn project() -> WorkProject {
        WorkProject {
            project_id:               "fabro".to_string(),
            display_name:             "fabro".to_string(),
            tracker_provider:         "github".to_string(),
            tracker_repo:             "denkhaus/work".to_string(),
            target_repo:              Some("denkhaus/fabro".to_string()),
            project_label:            "project:fabro".to_string(),
            lane_labels:              BTreeMap::from([(
                "develop".to_string(),
                "lane:develop".to_string(),
            )]),
            claim_label_prefix:       "claimed".to_string(),
            claim_comment_enabled:    false,
            backpressure_open_pr_cap: 0,
        }
    }

    #[tokio::test]
    async fn the_item_thread_resolves_live_recovers_released_and_transfers() {
        let state = crate::test_support::test_app_state();
        let store = state.work_store();
        store.upsert_project(&project()).await.unwrap();

        // Live claim: the source still holds the item.
        let source = RunId::new();
        let claim = match store
            .grant(&GrantRequest {
                project_id: "fabro".to_string(),
                item_key:   "denkhaus/work#1312".to_string(),
                run_id:     source,
                lane:       "develop".to_string(),
            })
            .await
            .unwrap()
        {
            GrantOutcome::Granted(claim) => claim,
            other => panic!("expected a grant, got {other:?}"),
        };
        let thread = source_item_thread(&state, &source)
            .await
            .expect("the live claim names the thread");
        assert_eq!(thread.item_key, "denkhaus/work#1312");

        // The parked case: the claim released, the association must
        // recover from the ledger's history (the 2026-11-14 evidence).
        store.release(&claim.claim_id, "parked").await.unwrap();
        let thread = source_item_thread(&state, &source)
            .await
            .expect("the latest claim record recovers the thread");
        assert_eq!(thread.item_key, "denkhaus/work#1312");
        assert_eq!(thread.lane, "develop");

        // The transfer: no live source claim left, so the successor
        // simply receives the grant.
        let successor = RunId::new();
        transfer_item_thread(&state, &source, &successor, &thread).await;
        let held = store
            .active_claim_for_run(&successor)
            .await
            .unwrap()
            .expect("the successor holds the item");
        assert_eq!(held.item_key, "denkhaus/work#1312");
        assert_eq!(held.state, ClaimState::Granted);
    }

    #[tokio::test]
    async fn the_transfer_releases_a_live_source_claim_before_granting() {
        let state = crate::test_support::test_app_state();
        let store = state.work_store();
        store.upsert_project(&project()).await.unwrap();

        let source = RunId::new();
        match store
            .grant(&GrantRequest {
                project_id: "fabro".to_string(),
                item_key:   "denkhaus/work#1313".to_string(),
                run_id:     source,
                lane:       "develop".to_string(),
            })
            .await
            .unwrap()
        {
            GrantOutcome::Granted(_) => {}
            other => panic!("expected a grant, got {other:?}"),
        }
        let thread = source_item_thread(&state, &source)
            .await
            .expect("the live claim names the thread");

        let successor = RunId::new();
        transfer_item_thread(&state, &source, &successor, &thread).await;

        // The source's claim is gone, the successor's is live: the
        // handoff is atomic from the item's point of view.
        assert!(
            store.active_claim_for_run(&source).await.unwrap().is_none(),
            "the source no longer holds the item"
        );
        let held = store
            .active_claim_for_run(&successor)
            .await
            .unwrap()
            .expect("the successor holds the item");
        assert_eq!(held.item_key, "denkhaus/work#1313");
    }

    #[tokio::test]
    async fn a_source_without_any_claim_names_no_thread() {
        let state = crate::test_support::test_app_state();
        let stranger = RunId::new();
        assert!(source_item_thread(&state, &stranger).await.is_none());
    }
}
