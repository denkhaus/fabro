//! Project-scoped work items and the server-owned claim ledger (ADR-0032).
//!
//! The tracker provider owns work-item content; this surface owns the
//! project registry and the atomic claim mutex. A claim is unique per
//! `(project, item)` and per run while it is non-terminal.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum_extra::extract::Query as ExtraQuery;
use fabro_api::types::{
    WorkClaim as ApiWorkClaim, WorkClaimEvent as ApiWorkClaimEvent, WorkClaimEventListResponse,
    WorkClaimGrantOutcome, WorkClaimGrantRequest, WorkClaimGrantResponse, WorkClaimListResponse,
    WorkClaimNextOutcome, WorkClaimNextRequest, WorkClaimNextResponse, WorkClaimReleaseRequest,
    WorkClaimState, WorkClaimTransitionRequest, WorkItem as ApiWorkItem, WorkItemCloseRequest,
    WorkItemCreateRequest, WorkItemListResponse, WorkProject as ApiWorkProject,
    WorkProjectListResponse, WorkProjectUpsertRequest,
};
use fabro_store::platform_records::{PlatformRecord, WorkClaimedRecord, WorkReleasedRecord};
use fabro_types::RunId;
use fabro_work::{
    ClaimState, GrantOutcome, GrantRequest, TrackerProvider, WorkClaim, WorkClaimEvent, WorkItem,
    WorkItemDraft, WorkItemFilter, WorkProject,
};

use super::super::{
    ApiError, AppState, IntoResponse, Json, Path, Response, Router, State, StatusCode, get, post,
};
use crate::principal_middleware::{RequiredRunManagementActor, RequiredUser};

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/projects", get(list_projects))
        .route(
            "/projects/{projectId}",
            get(retrieve_project).put(upsert_project),
        )
        .route(
            "/projects/{projectId}/claims",
            get(list_claims).post(grant_claim),
        )
        .route(
            "/projects/{projectId}/work-items",
            get(list_work_items).post(create_work_item),
        )
        .route(
            "/projects/{projectId}/work-items/close",
            post(close_work_item),
        )
        .route("/projects/{projectId}/next", post(claim_next))
        .route("/projects/{projectId}/work-item", get(retrieve_work_item))
        .route("/claims/{claimId}", get(retrieve_claim))
        .route("/claims/{claimId}/transition", post(transition_claim))
        .route("/claims/{claimId}/release", post(release_claim))
        .route("/claims/{claimId}/events", get(list_claim_events))
}

async fn list_projects(
    _auth: RequiredUser,
    State(state): State<Arc<AppState>>,
) -> Result<Response, ApiError> {
    let projects = state.work_store().projects().await.map_err(store_error)?;
    let response = WorkProjectListResponse {
        data: projects.into_iter().map(api_project).collect(),
    };
    Ok(Json(response).into_response())
}

async fn retrieve_project(
    _auth: RequiredUser,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> Result<Response, ApiError> {
    let project = state
        .work_store()
        .project(&project_id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| not_found("work project", &project_id))?;
    Ok(Json(api_project(project)).into_response())
}

async fn upsert_project(
    _auth: RequiredUser,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(request): Json<WorkProjectUpsertRequest>,
) -> Result<Response, ApiError> {
    validate_project_id(&project_id)?;
    validate_project(&request)?;
    let project = WorkProject {
        project_id:            project_id.clone(),
        display_name:          request.display_name,
        tracker_provider:      request.tracker_provider,
        tracker_repo:          request.tracker_repo,
        project_label:         request.project_label,
        lane_labels:           request.lane_labels.into_iter().collect::<BTreeMap<_, _>>(),
        claim_label_prefix:    request
            .claim_label_prefix
            .unwrap_or_else(|| "fabro:run:".to_string()),
        claim_comment_enabled: request.claim_comment_enabled.unwrap_or(false),
    };
    state
        .work_store()
        .upsert_project(&project)
        .await
        .map_err(store_error)?;
    Ok(Json(api_project(project)).into_response())
}

async fn list_claims(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> Result<Response, ApiError> {
    if state
        .work_store()
        .project(&project_id)
        .await
        .map_err(store_error)?
        .is_none()
    {
        return Err(not_found("work project", &project_id));
    }
    let claims = state
        .work_store()
        .active_claims(&project_id)
        .await
        .map_err(store_error)?;
    let response = WorkClaimListResponse {
        data: claims
            .into_iter()
            .map(api_claim)
            .collect::<Result<_, _>>()?,
    };
    Ok(Json(response).into_response())
}

async fn grant_claim(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(request): Json<WorkClaimGrantRequest>,
) -> Result<Response, ApiError> {
    let run_id = request
        .run_id
        .parse::<RunId>()
        .map_err(|err| ApiError::bad_request(format!("invalid runId: {err}")))?;
    let outcome = state
        .work_store()
        .grant(&GrantRequest {
            project_id: project_id.clone(),
            item_key: request.item_key,
            run_id,
            lane: request.lane,
        })
        .await
        .map_err(store_error)?;
    let (outcome, claim) = match outcome {
        GrantOutcome::Granted(claim) => (WorkClaimGrantOutcome::Granted, claim),
        GrantOutcome::AlreadyHeld(claim) => (WorkClaimGrantOutcome::AlreadyHeld, claim),
        GrantOutcome::Conflict(claim) => (WorkClaimGrantOutcome::Conflict, claim),
    };
    Ok(Json(WorkClaimGrantResponse {
        outcome,
        claim: api_claim(claim)?,
    })
    .into_response())
}

async fn retrieve_claim(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(claim_id): Path<String>,
) -> Result<Response, ApiError> {
    validate_claim_id(&claim_id)?;
    let claim = state
        .work_store()
        .claim(&claim_id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| not_found("work claim", &claim_id))?;
    Ok(Json(api_claim(claim)?).into_response())
}

async fn transition_claim(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(claim_id): Path<String>,
    Json(request): Json<WorkClaimTransitionRequest>,
) -> Result<Response, ApiError> {
    validate_claim_id(&claim_id)?;
    let claim = state
        .work_store()
        .transition(
            &claim_id,
            store_state(request.state),
            request.reason.as_deref(),
            request.pr_url.as_deref(),
        )
        .await
        .map_err(store_error)?;
    if claim.state.holds_item() {
        if let Ok(Some(project)) = state.work_store().project(&claim.project_id).await {
            best_effort_clear_claim(&state, &project, &claim).await;
        }
    } else {
        append_work_record(
            &state,
            claim.run_id,
            PlatformRecord::WorkReleased(WorkReleasedRecord {
                claim_id:   claim.claim_id.clone(),
                project_id: claim.project_id.clone(),
                item_key:   claim.item_key.clone(),
                reason:     claim
                    .release_reason
                    .clone()
                    .unwrap_or_else(|| claim.state.as_str().to_string()),
            }),
        )
        .await;
    }
    Ok(Json(api_claim(claim)?).into_response())
}

async fn release_claim(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(claim_id): Path<String>,
    Json(request): Json<WorkClaimReleaseRequest>,
) -> Result<Response, ApiError> {
    validate_claim_id(&claim_id)?;
    let claim = state
        .work_store()
        .release(&claim_id, &request.reason)
        .await
        .map_err(store_error)?;
    if let Ok(Some(project)) = state.work_store().project(&claim.project_id).await {
        best_effort_clear_claim(&state, &project, &claim).await;
    }
    append_work_record(
        &state,
        claim.run_id,
        PlatformRecord::WorkReleased(WorkReleasedRecord {
            claim_id:   claim.claim_id.clone(),
            project_id: claim.project_id.clone(),
            item_key:   claim.item_key.clone(),
            reason:     format!("released: {}", request.reason),
        }),
    )
    .await;
    Ok(Json(api_claim(claim)?).into_response())
}

async fn list_claim_events(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(claim_id): Path<String>,
) -> Result<Response, ApiError> {
    validate_claim_id(&claim_id)?;
    if state
        .work_store()
        .claim(&claim_id)
        .await
        .map_err(store_error)?
        .is_none()
    {
        return Err(not_found("work claim", &claim_id));
    }
    let events = state
        .work_store()
        .events(&claim_id)
        .await
        .map_err(store_error)?;
    let response = WorkClaimEventListResponse {
        data: events.into_iter().map(api_event).collect(),
    };
    Ok(Json(response).into_response())
}

fn validate_project_id(project_id: &str) -> Result<(), ApiError> {
    let valid = !project_id.is_empty()
        && project_id.len() <= 63
        && project_id
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && project_id
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-');
    if valid {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "projectId must match ^[a-z0-9][a-z0-9-]{0,62}$",
        ))
    }
}

fn validate_claim_id(claim_id: &str) -> Result<(), ApiError> {
    uuid::Uuid::parse_str(claim_id)
        .map(|_| ())
        .map_err(|_| ApiError::bad_request("claimId must be a UUID"))
}

fn validate_project(request: &WorkProjectUpsertRequest) -> Result<(), ApiError> {
    for (field, value) in [
        ("displayName", &request.display_name),
        ("trackerProvider", &request.tracker_provider),
        ("trackerRepo", &request.tracker_repo),
        ("projectLabel", &request.project_label),
    ] {
        if value.trim().is_empty() {
            return Err(ApiError::bad_request(format!("{field} must not be empty")));
        }
    }
    if request
        .claim_label_prefix
        .as_deref()
        .is_some_and(|prefix| prefix.trim().is_empty())
    {
        return Err(ApiError::bad_request("claimLabelPrefix must not be empty"));
    }
    Ok(())
}

fn not_found(resource: &str, id: &str) -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, format!("{resource} not found: {id}"))
}

fn store_error(error: fabro_work::Error) -> ApiError {
    match error {
        fabro_work::Error::WorkProjectNotFound(id) => not_found("work project", &id),
        fabro_work::Error::WorkClaimNotFound(id) => not_found("work claim", &id),
        fabro_work::Error::InvalidClaimTransition { from, to } => ApiError::new(
            StatusCode::CONFLICT,
            format!("invalid work claim transition: {from} -> {to}"),
        ),
        fabro_work::Error::WorkClaimConflict(message) => {
            ApiError::new(StatusCode::CONFLICT, message)
        }
        other => {
            tracing::error!(error = %other, "work store operation failed");
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "work store operation failed",
            )
        }
    }
}

async fn list_work_items(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    ExtraQuery(params): ExtraQuery<WorkItemQuery>,
) -> Result<Response, ApiError> {
    let project = load_project(&state, &project_id).await?;
    let tracker = tracker_for(&state, &project).await?;
    let items = tracker
        .list_open(&project, WorkItemFilter {
            lane:  params.lane,
            limit: params.limit.unwrap_or(50).clamp(1, 100) as usize,
        })
        .await
        .map_err(tracker_failed)?;
    let response = WorkItemListResponse {
        data: items.into_iter().map(api_item).collect(),
    };
    Ok(Json(response).into_response())
}

#[derive(Debug, Default, serde::Deserialize)]
struct WorkItemQuery {
    lane:  Option<String>,
    limit: Option<u32>,
}

async fn create_work_item(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(request): Json<WorkItemCreateRequest>,
) -> Result<Response, ApiError> {
    if request.title.trim().is_empty() {
        return Err(ApiError::bad_request("title must not be empty"));
    }
    if request.body.trim().is_empty() {
        return Err(ApiError::bad_request("body must not be empty"));
    }
    let project = load_project(&state, &project_id).await?;
    let tracker = tracker_for(&state, &project).await?;
    let mut labels = vec![project.project_label.clone()];
    labels.extend(request.labels);
    labels.sort();
    labels.dedup();
    let item = tracker
        .create(&project, WorkItemDraft {
            title: request.title,
            body: request.body,
            labels,
        })
        .await
        .map_err(tracker_failed)?;
    Ok((StatusCode::CREATED, Json(api_item(item))).into_response())
}

async fn close_work_item(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(request): Json<WorkItemCloseRequest>,
) -> Result<Response, ApiError> {
    let project = load_project(&state, &project_id).await?;
    let tracker = tracker_for(&state, &project).await?;
    tracker
        .close(&project, &request.item_key, &request.reason)
        .await
        .map_err(tracker_failed)?;
    // A closed item releases any live claim (DB first, projection second).
    if let Some(claim) = state
        .work_store()
        .active_claim_for_item(&project_id, &request.item_key)
        .await
        .map_err(store_error)?
    {
        state
            .work_store()
            .release(&claim.claim_id, "item closed")
            .await
            .map_err(store_error)?;
        best_effort_clear_claim(&state, &project, &claim).await;
    }
    let item = tracker
        .get(&project, &request.item_key)
        .await
        .map_err(tracker_failed)?
        .ok_or_else(|| not_found("work item", &request.item_key))?;
    Ok(Json(api_item(item)).into_response())
}

async fn claim_next(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(request): Json<WorkClaimNextRequest>,
) -> Result<Response, ApiError> {
    let run_id = request
        .run_id
        .parse::<RunId>()
        .map_err(|err| ApiError::bad_request(format!("invalid runId: {err}")))?;
    let project = load_project(&state, &project_id).await?;
    let tracker = state
        .work_tracker_for_project(&project)
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("tracker provider is not available: {error:#}"),
            )
        })?;
    let items = tracker
        .list_open(&project, WorkItemFilter {
            lane:  Some(request.lane.clone()),
            limit: 50,
        })
        .await
        .map_err(tracker_failed)?;
    let mut claimed: std::collections::HashSet<String> = state
        .work_store()
        .active_claims(&project_id)
        .await
        .map_err(store_error)?
        .into_iter()
        .map(|claim| claim.item_key)
        .collect();

    for item in items {
        if claimed.contains(&item.item_key) {
            continue;
        }
        match state
            .work_store()
            .grant(&GrantRequest {
                project_id: project_id.clone(),
                item_key: item.item_key.clone(),
                run_id,
                lane: request.lane.clone(),
            })
            .await
            .map_err(store_error)?
        {
            GrantOutcome::Granted(claim) => {
                // The claim committed; the tracker projection is eventual. A
                // failed projection enqueues the durable outbox job.
                if let Err(error) = tracker
                    .project_claim(&project, &item.item_key, &claim.run_id.to_string())
                    .await
                {
                    tracing::warn!(
                        claim_id = %claim.claim_id,
                        error = %error,
                        "tracker claim projection failed; queued for retry"
                    );
                    let _ = state
                        .work_store()
                        .enqueue(
                            "claim.projected",
                            serde_json::json!({
                                "claim_id": claim.claim_id,
                                "project_id": claim.project_id,
                                "item_key": claim.item_key,
                                "run_id": claim.run_id.to_string(),
                            }),
                            &format!("claim-projected:{}:granted", claim.claim_id),
                        )
                        .await;
                }
                append_work_record(
                    &state,
                    run_id,
                    PlatformRecord::WorkClaimed(WorkClaimedRecord {
                        claim_id:   claim.claim_id.clone(),
                        project_id: claim.project_id.clone(),
                        item_key:   claim.item_key.clone(),
                        lane:       claim.lane.clone(),
                    }),
                )
                .await;
                return Ok(Json(WorkClaimNextResponse {
                    outcome: WorkClaimNextOutcome::Granted,
                    claim:   Some(api_claim(claim)?),
                    item:    Some(api_item(item)),
                })
                .into_response());
            }
            GrantOutcome::AlreadyHeld(claim) => {
                let held_item = tracker
                    .get(&project, &claim.item_key)
                    .await
                    .ok()
                    .flatten()
                    .or(Some(item));
                return Ok(Json(WorkClaimNextResponse {
                    outcome: WorkClaimNextOutcome::AlreadyHeld,
                    claim:   Some(api_claim(claim)?),
                    item:    held_item.map(api_item),
                })
                .into_response());
            }
            GrantOutcome::Conflict(_) => {
                claimed.insert(item.item_key.clone());
            }
        }
    }

    // The loop skips every candidate when this run already holds an item.
    if let Some(claim) = state
        .work_store()
        .active_claim_for_run(&run_id)
        .await
        .map_err(store_error)?
    {
        let held_item = tracker.get(&project, &claim.item_key).await.ok().flatten();
        return Ok(Json(WorkClaimNextResponse {
            outcome: WorkClaimNextOutcome::AlreadyHeld,
            claim:   Some(api_claim(claim)?),
            item:    held_item.map(api_item),
        })
        .into_response());
    }

    Ok(Json(WorkClaimNextResponse {
        outcome: WorkClaimNextOutcome::Empty,
        claim:   None,
        item:    None,
    })
    .into_response())
}

/// Append a work record to the run's stream, best effort: the claim in the
/// store is the authority; the record is audit. A failed append is logged,
/// never fatal to the claim path.
async fn append_work_record(state: &Arc<AppState>, run_id: RunId, record: PlatformRecord) {
    if let Err(error) = super::super::run_records::append(state, run_id, record).await {
        tracing::warn!(run_id = %run_id, error = %error, "work record append failed");
    }
}

async fn retrieve_work_item(
    _auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    ExtraQuery(params): ExtraQuery<WorkItemKeyQuery>,
) -> Result<Response, ApiError> {
    let item_key = params
        .item_key
        .ok_or_else(|| ApiError::bad_request("itemKey is required"))?;
    let project = load_project(&state, &project_id).await?;
    let tracker = tracker_for(&state, &project).await?;
    let item = tracker
        .get(&project, &item_key)
        .await
        .map_err(tracker_failed)?
        .ok_or_else(|| not_found("work item", &item_key))?;
    Ok(Json(api_item(item)).into_response())
}

#[derive(Debug, Default, serde::Deserialize)]
struct WorkItemKeyQuery {
    #[serde(rename = "itemKey")]
    item_key: Option<String>,
}

async fn load_project(state: &Arc<AppState>, project_id: &str) -> Result<WorkProject, ApiError> {
    state
        .work_store()
        .project(project_id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| not_found("work project", project_id))
}

async fn tracker_for(
    state: &Arc<AppState>,
    project: &WorkProject,
) -> Result<Arc<dyn TrackerProvider>, ApiError> {
    state
        .work_tracker_for_project(project)
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("tracker provider is not available: {error:#}"),
            )
        })
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "map_err hands over ownership; a by-value error keeps call sites closure-free"
)]
fn tracker_failed(error: anyhow::Error) -> ApiError {
    ApiError::new(
        StatusCode::BAD_GATEWAY,
        format!("tracker provider failed: {error:#}"),
    )
}

/// Clear the run-id claim label after a release, best effort: the claim
/// record is already terminal; a failed projection enqueues the outbox job.
async fn best_effort_clear_claim(state: &Arc<AppState>, project: &WorkProject, claim: &WorkClaim) {
    let tracker = match state.work_tracker_for_project(project).await {
        Ok(tracker) => tracker,
        Err(error) => {
            tracing::warn!(error = %error, "tracker unavailable for claim clear");
            return;
        }
    };
    if let Err(error) = tracker
        .clear_claim(project, &claim.item_key, &claim.run_id.to_string())
        .await
    {
        tracing::warn!(
            claim_id = %claim.claim_id,
            error = %error,
            "tracker claim clear failed; queued for retry"
        );
        let _ = state
            .work_store()
            .enqueue(
                "claim.cleared",
                serde_json::json!({
                    "claim_id": claim.claim_id,
                    "project_id": claim.project_id,
                    "item_key": claim.item_key,
                    "run_id": claim.run_id.to_string(),
                }),
                &format!("claim-cleared:{}", claim.claim_id),
            )
            .await;
    }
}

fn api_item(item: WorkItem) -> ApiWorkItem {
    ApiWorkItem {
        body:       item.body,
        item_key:   item.item_key,
        labels:     item.labels,
        priority:   item.priority,
        state:      item.state,
        title:      item.title,
        updated_at: item.updated_at,
        url:        item.url,
    }
}

fn api_project(project: WorkProject) -> ApiWorkProject {
    ApiWorkProject {
        claim_comment_enabled: project.claim_comment_enabled,
        claim_label_prefix:    project.claim_label_prefix,
        display_name:          project.display_name,
        lane_labels:           project.lane_labels.into_iter().collect(),
        project_id:            project.project_id,
        project_label:         project.project_label,
        tracker_provider:      project.tracker_provider,
        tracker_repo:          project.tracker_repo,
    }
}

fn api_claim(claim: WorkClaim) -> Result<ApiWorkClaim, ApiError> {
    Ok(ApiWorkClaim {
        claim_id:       uuid::Uuid::parse_str(&claim.claim_id).map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "stored claim has an invalid id",
            )
        })?,
        granted_at:     claim.granted_at,
        item_key:       claim.item_key,
        lane:           claim.lane,
        pr_url:         claim.pr_url,
        project_id:     claim.project_id,
        release_reason: claim.release_reason,
        released_at:    claim.released_at,
        run_id:         claim.run_id.to_string(),
        state:          api_state(claim.state),
        updated_at:     claim.updated_at,
    })
}

fn api_event(event: WorkClaimEvent) -> ApiWorkClaimEvent {
    ApiWorkClaimEvent {
        at:         event.at,
        claim_id:   event.claim_id,
        from_state: event.from_state.map(api_state),
        reason:     event.reason,
        run_id:     event.run_id.to_string(),
        seq:        event.seq,
        to_state:   api_state(event.to_state),
    }
}

fn api_state(state: ClaimState) -> WorkClaimState {
    match state {
        ClaimState::Granted => WorkClaimState::Granted,
        ClaimState::Active => WorkClaimState::Active,
        ClaimState::AwaitingMerge => WorkClaimState::AwaitingMerge,
        ClaimState::Landed => WorkClaimState::Landed,
        ClaimState::Released => WorkClaimState::Released,
    }
}

fn store_state(state: WorkClaimState) -> ClaimState {
    match state {
        WorkClaimState::Granted => ClaimState::Granted,
        WorkClaimState::Active => ClaimState::Active,
        WorkClaimState::AwaitingMerge => ClaimState::AwaitingMerge,
        WorkClaimState::Landed => ClaimState::Landed,
        WorkClaimState::Released => ClaimState::Released,
    }
}
