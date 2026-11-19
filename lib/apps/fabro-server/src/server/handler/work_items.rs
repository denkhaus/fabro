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
    WorkItemCloseResponse, WorkItemCommentList, WorkItemCreateRequest, WorkItemLaneRequest,
    WorkItemListResponse, WorkItemLockRequest, WorkItemRadiusResponse, WorkItemUpdateRequest,
    WorkObservationLinkOutcome, WorkObservationLinkRequest, WorkObservationLinkResponse,
    WorkObservationUnlinkResponse, WorkProject as ApiWorkProject, WorkProjectListResponse,
    WorkProjectUpsertRequest,
};
use fabro_store::platform_records::{
    PlatformRecord, PlatformRecordKind, WorkClaimedRecord, WorkLandedRecord, WorkReleasedRecord,
};
use fabro_types::RunId;
use fabro_work::{
    ClaimState, GrantOutcome, GrantRequest, TrackerProvider, WorkClaim, WorkClaimEvent, WorkItem,
    WorkItemDraft, WorkItemFilter, WorkItemStateFilter, WorkProject,
};

use super::super::{
    ApiError, AppState, IntoResponse, Json, Path, Response, Router, State, StatusCode, get, post,
    run_records,
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
        .route("/projects/{projectId}/work-items/item", get(get_work_item))
        .route(
            "/projects/{projectId}/work-items/close",
            post(close_work_item),
        )
        .route(
            "/projects/{projectId}/work-items/link",
            post(link_observation),
        )
        .route(
            "/projects/{projectId}/work-items/{itemKey}/radius",
            get(get_work_item_radius),
        )
        .route(
            "/projects/{projectId}/work-items/{itemKey}/comments",
            get(list_work_item_comments),
        )
        .route(
            "/projects/{projectId}/work-items/unlink",
            post(unlink_observation),
        )
        .route(
            "/projects/{projectId}/work-items/update",
            post(update_work_item),
        )
        .route(
            "/projects/{projectId}/work-items/lane",
            post(lane_work_item),
        )
        .route(
            "/projects/{projectId}/work-items/lock",
            post(lock_work_item),
        )
        .route(
            "/projects/{projectId}/work-items/unlock",
            post(unlock_work_item),
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
    if project_id == "self" {
        return Err(ApiError::bad_request(
            "project id `self` is reserved for run-scoped project resolution",
        ));
    }
    validate_project(&request)?;
    let project = WorkProject {
        project_id:               project_id.clone(),
        display_name:             request.display_name,
        tracker_provider:         request.tracker_provider,
        tracker_repo:             request.tracker_repo,
        target_repo:              request.target_repo,
        project_label:            request.project_label,
        lane_labels:              request.lane_labels.into_iter().collect::<BTreeMap<_, _>>(),
        claim_label_prefix:       request
            .claim_label_prefix
            .unwrap_or_else(|| fabro_work::DEFAULT_CLAIM_LABEL_PREFIX.to_owned()),
        claim_comment_enabled:    request.claim_comment_enabled.unwrap_or(false),
        backpressure_open_pr_cap: request.backpressure_open_pr_cap.unwrap_or(0),
    };
    state
        .work_store()
        .upsert_project(&project)
        .await
        .map_err(store_error)?;
    Ok(Json(api_project(project)).into_response())
}

async fn list_claims(
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> Result<Response, ApiError> {
    // Workers read their own project's ledger only (denkhaus/work#991).
    let caller = resolve_caller_project(&state, &auth, &project_id).await?;
    let claims = state
        .work_store()
        .active_claims(&caller.project().project_id)
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
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(request): Json<WorkClaimGrantRequest>,
) -> Result<Response, ApiError> {
    let run_id = request
        .run_id
        .parse::<RunId>()
        .map_err(|err| ApiError::bad_request(format!("invalid runId: {err}")))?;
    let caller = resolve_caller_project(&state, &auth, &project_id).await?;
    // Workers grant into their own project for their OWN run only
    // (denkhaus/work#991): a foreign body runId is refused loudly.
    let run_id = bind_granted_run_id(&caller, run_id)?;
    let project = caller.project();
    let outcome = state
        .work_store()
        .grant(&GrantRequest {
            project_id: project.project_id.clone(),
            item_key: request.item_key.clone(),
            run_id,
            lane: request.lane,
        })
        .await
        .map_err(store_error)?;
    let (outcome, claim, item_key) = match outcome {
        GrantOutcome::Granted(claim) => {
            let key = claim.item_key.clone();
            (WorkClaimGrantOutcome::Granted, claim, key)
        }
        GrantOutcome::AlreadyHeld(claim) => {
            let key = claim.item_key.clone();
            (WorkClaimGrantOutcome::AlreadyHeld, claim, key)
        }
        GrantOutcome::Conflict(claim) => {
            let key = claim.item_key.clone();
            (WorkClaimGrantOutcome::Conflict, claim, key)
        }
    };
    // The granted item rides along (tool parity with /next: one call
    // returns claim + item; null when the tracker cannot serve it).
    let mut item = None;
    if matches!(
        outcome,
        WorkClaimGrantOutcome::Granted | WorkClaimGrantOutcome::AlreadyHeld
    ) {
        if let Ok(tracker) = tracker_for(&state, project).await {
            item = tracker.get(project, &item_key).await.ok().flatten();
        }
    }
    Ok(Json(WorkClaimGrantResponse {
        outcome,
        claim: api_claim(claim)?,
        item: item.map(api_item),
    })
    .into_response())
}

async fn retrieve_claim(
    auth: RequiredRunManagementActor,
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
    bind_claim_to_caller(&auth, &claim)?;
    Ok(Json(api_claim(claim)?).into_response())
}

async fn transition_claim(
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(claim_id): Path<String>,
    Json(request): Json<WorkClaimTransitionRequest>,
) -> Result<Response, ApiError> {
    validate_claim_id(&claim_id)?;
    // The binding check runs BEFORE the store mutation: a foreign claim
    // is refused without side effects (denkhaus/work#991).
    load_bound_claim(&state, &auth, &claim_id).await?;
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
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(claim_id): Path<String>,
    Json(request): Json<WorkClaimReleaseRequest>,
) -> Result<Response, ApiError> {
    validate_claim_id(&claim_id)?;
    // The binding check runs BEFORE the store mutation: a foreign claim
    // is refused without side effects (denkhaus/work#991).
    load_bound_claim(&state, &auth, &claim_id).await?;
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
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(claim_id): Path<String>,
) -> Result<Response, ApiError> {
    validate_claim_id(&claim_id)?;
    // Workers read their own claims' event streams only
    // (denkhaus/work#991).
    let claim = state
        .work_store()
        .claim(&claim_id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| not_found("work claim", &claim_id))?;
    bind_claim_to_caller(&auth, &claim)?;
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
        fabro_work::Error::WorkItemLocked(item_key) => ApiError::with_code(
            StatusCode::CONFLICT,
            format!(
                "item {item_key} carries an operator lock — claiming is refused until an \
                 operator lifts it (`fabro work unlock {item_key}`)"
            ),
            "item_locked",
        ),
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
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    ExtraQuery(params): ExtraQuery<WorkItemQuery>,
) -> Result<Response, ApiError> {
    // Workers list their own project's items only — foreign reads are
    // refused, not just foreign writes (denkhaus/work#991, ADR-0019).
    let caller = resolve_caller_project(&state, &auth, &project_id).await?;
    let project = caller.project();
    let tracker = tracker_for(&state, project).await?;
    // Which states the listing enumerates (work#1476): the default stays
    // OPEN — every pre-#1476 caller's answer, unchanged; `all` is the
    // census read (the friction-score swap). Unknown values are refused
    // loudly, never silently narrowed.
    let state_filter = match params.state.as_deref() {
        None | Some("open") => WorkItemStateFilter::Open,
        Some("closed") => WorkItemStateFilter::Closed,
        Some("all") => WorkItemStateFilter::All,
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "unknown state '{other}' (open | closed | all)"
            )));
        }
    };
    // The cursor (work#1476) names the page to serve: the previous
    // response's `nextCursor` verbatim, the first page when absent. The
    // token is the 1-based page number — the same walk the tracker
    // provider serves, opaque to the client. It overrides an explicit
    // `page` so a walker can never double-read a page boundary.
    let page = match params.cursor.as_deref().map(str::trim) {
        None | Some("") => params.page.unwrap_or(1).max(1),
        Some(cursor) => match cursor.parse::<u32>() {
            Ok(n) if n >= 1 => n,
            _ => {
                return Err(ApiError::bad_request(
                    "invalid cursor — pass the previous response's nextCursor verbatim, \
                     or omit it for the first page",
                ));
            }
        },
    };
    let page_size = params.limit.unwrap_or(50).clamp(1, 100) as usize;
    let sort = match params.sort.as_deref() {
        None => None,
        Some("priority") => Some(("priority", false)),
        Some("-priority") => Some(("priority", true)),
        Some("updated") => Some(("updated", false)),
        Some("-updated") => Some(("updated", true)),
        Some(other) => {
            return Err(ApiError::bad_request(format!(
                "unknown sort key '{other}' (priority | -priority | updated | -updated)"
            )));
        }
    };
    // The pager's truthful total (denkhaus/work#1296): the tracker's
    // counting read names the number of items the filter matches,
    // independent of paging — the page-window estimate is gone.
    let page = usize::try_from(page).unwrap_or(1);
    let mut total: Option<u64> = None;
    let items = match sort {
        None => {
            let page = tracker
                .list_with_total(project, WorkItemFilter {
                    lane: params.lane,
                    limit: page_size,
                    query: params.query,
                    label: params.label,
                    page,
                    state: state_filter,
                })
                .await
                .map_err(tracker_failed)?;
            total = page.total;
            page.items
        }
        Some((key, inverted)) => {
            // Sorting applies before paging: accumulate enough tracker
            // pages to cover the requested one (bounded to three tracker
            // pages of 100 — the project's scale), sort, slice. The
            // total rides the FIRST tracker page's counting read; the
            // later pages fetch items only (the count is page-invariant).
            const TRACKER_PAGE: usize = 100;
            let pages_needed = (page * page_size).div_ceil(TRACKER_PAGE);
            let mut accumulated: Vec<fabro_work::WorkItem> = Vec::new();
            for tracker_page in 1..=pages_needed.min(3) {
                let filter = WorkItemFilter {
                    lane:  params.lane.clone(),
                    limit: TRACKER_PAGE,
                    query: params.query.clone(),
                    label: params.label.clone(),
                    page:  tracker_page,
                    state: state_filter,
                };
                let batch = if tracker_page == 1 {
                    let page = tracker
                        .list_with_total(project, filter)
                        .await
                        .map_err(tracker_failed)?;
                    total = page.total;
                    page.items
                } else {
                    tracker
                        .list(project, filter)
                        .await
                        .map_err(tracker_failed)?
                };
                let exhausted = batch.len() < TRACKER_PAGE;
                accumulated.extend(batch);
                if exhausted {
                    break;
                }
            }
            accumulated.sort_by(|a, b| {
                let ordering = match key {
                    "updated" => a.updated_at.cmp(&b.updated_at),
                    _ => a
                        .priority
                        .cmp(&b.priority)
                        .then(a.updated_at.cmp(&b.updated_at)),
                };
                if inverted {
                    ordering.reverse()
                } else {
                    ordering
                }
            });
            let start = (page - 1) * page_size;
            accumulated
                .into_iter()
                .skip(start)
                .take(page_size)
                .collect::<Vec<_>>()
        }
    };
    // The lock badge overlay (denkhaus/work#1420): the lock lives in the
    // store beside the claims, so the tracker-shaped items are stamped
    // here — the operator sees the hold in the same listing that shows
    // the lane.
    let locked_items: std::collections::HashSet<String> = state
        .work_store()
        .locked_items(&project.project_id)
        .await
        .map_err(store_error)?
        .into_iter()
        .collect();
    // The next-page cursor (work#1476), the full-page heuristic: a page
    // that filled its page size MAY have more (the caller walks until a
    // short page or a null cursor — the `gh --paginate` posture); a
    // short page is the last one, whatever the total said.
    let next_cursor = if items.len() == page_size {
        Some(page.saturating_add(1).to_string())
    } else {
        None
    };
    let response = WorkItemListResponse {
        data:  items
            .into_iter()
            .map(|item| {
                let locked = locked_items.contains(&item.item_key);
                stamp_lock(api_item(item), locked)
            })
            .collect(),
        total: total.map(|total| total.min(i64::MAX.unsigned_abs()).cast_signed()),
        next_cursor,
    };
    Ok(Json(response).into_response())
}

#[derive(Debug, Default, serde::Deserialize)]
struct WorkItemQuery {
    lane:  Option<String>,
    limit: Option<u32>,
    query: Option<String>,
    /// Raw tracker label name, filtered beside the project/lane labels
    /// (denkhaus/work#1001).
    label: Option<String>,
    /// 1-based results page; `limit` is the page size (denkhaus/work#1001).
    page:  Option<u32>,
    /// Server-side sort key: `priority` or `updated`, `-` inverts.
    /// Applies before paging: the handler accumulates tracker pages
    /// (bounded), sorts, and slices the requested page.
    sort:  Option<String>,
    /// Which states the listing enumerates: `open` (the default),
    /// `closed`, or `all` — the census read (work#1476).
    state: Option<String>,
    /// The previous response's opaque next-page token (work#1476);
    /// absent or empty starts at the first page. Overrides `page`.
    cursor: Option<String>,
}

/// One work item by key, including closed items (denkhaus/work#1235):
/// the list's text `query` searches title and body, so a key-addressed
/// fetch needs its own door — the web work view's detail read. The key
/// rides a QUERY parameter: it is opaque (`owner/repo#n`), and a slash
/// inside a path parameter is not routable (the path form 400'd on the
/// encoded slash in the first live proof, 2026-11-10).
#[derive(Debug, serde::Deserialize)]
struct WorkItemGetQuery {
    /// camelCase on the wire (the spec's `itemKey`) — the first
    /// multi-word query param on this surface.
    #[serde(rename = "itemKey")]
    item_key: String,
}

async fn get_work_item(
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    ExtraQuery(query): ExtraQuery<WorkItemGetQuery>,
) -> Result<Response, ApiError> {
    let caller = resolve_caller_project(&state, &auth, &project_id).await?;
    let project = caller.project();
    let tracker = tracker_for(&state, project).await?;
    let item = tracker
        .get(project, &query.item_key)
        .await
        .map_err(tracker_failed)?
        .ok_or_else(|| ApiError::not_found(format!("work item `{}` not found", query.item_key)))?;
    let locked = state
        .work_store()
        .item_lock(&project.project_id, &query.item_key)
        .await
        .map_err(store_error)?
        .is_some();
    Ok(Json(stamp_lock(api_item(item), locked)).into_response())
}

async fn create_work_item(
    auth: RequiredRunManagementActor,
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
    // The observation refs arrive as the canonical type (with_replacement)
    // — no parsing, only the u64→seq identity (ADR-0044: filing IS
    // linking; the links land beside the tracker create below).
    let observations = request.observations;
    let caller = resolve_caller_project(&state, &auth, &project_id).await?;
    let project = caller.project();
    let tracker = tracker_for(&state, project).await?;
    let mut labels = vec![project.project_label.clone()];
    labels.extend(request.labels);
    labels.sort();
    labels.dedup();
    let item = tracker
        .create(project, WorkItemDraft {
            title: request.title,
            body: request.body,
            labels,
        })
        .await
        .map_err(tracker_failed)?;
    // The link half (ADR-0044): the create door is the worker's only
    // link write — the rows land beside the tracker create, idempotently
    // (an already-linked pair stays one link; dedupe at the link). A
    // link failure fails the REQUEST (the item exists; the caller
    // retries with the same refs and the dedupe keeps the chain whole)
    // — never a silently unlinked filing, the break this door exists to
    // close. Each landed link is a write door of the triage read model
    // (work#1385): the referenced record's derived state re-derives
    // once and materializes beside the record.
    for reference in observations {
        state
            .work_store()
            .link_observation(
                &project.project_id,
                &item.item_key,
                &reference.run_id,
                reference.seq,
            )
            .await
            .map_err(|err| {
                tracing::error!(
                    error = ?err,
                    item_key = %item.item_key,
                    run_id = %reference.run_id,
                    seq = reference.seq,
                    "observation link write failed at create"
                );
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "The work item was created ({}), but linking its observation failed — \
                         retry the same create; the link dedupe keeps the chain whole.",
                        item.item_key
                    ),
                )
            })?;
        super::observation_triage::rederive_and_materialize(
            &state,
            &reference.run_id,
            reference.seq,
        )
        .await;
    }
    Ok((StatusCode::CREATED, Json(api_item(item))).into_response())
}

async fn close_work_item(
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(request): Json<WorkItemCloseRequest>,
) -> Result<Response, ApiError> {
    // A close without evidence is no close — refuse at the boundary for
    // every client (retro finding 2026-10-26), before any store or
    // tracker call.
    if request.reason.trim().is_empty() {
        return Err(ApiError::bad_request(
            "close reason must not be empty — a close carries its evidence",
        ));
    }
    let caller = resolve_caller_project(&state, &auth, &project_id).await?;
    let project = caller.project();
    // Worker closes carry the adjudicator exception (ADR-0039 stage E):
    // the ONLY worker closer is a run of the adjudicator workflow, and
    // only for an item whose claim's run the merger enabled
    // (`closable: enabled`). The own-claim arm is retired — a develop
    // run never closes its item (decision 1); the adjudicator is the
    // only closer (decision 6). The authorized close LANDS the claim
    // (merge-complete) instead of releasing it.
    let adjudicator_claim =
        adjudicator_close_authority(&state, &auth, project, &request.item_key).await?;
    // The closure-radius gate (ADR-0038): a claimed item whose radius is
    // open — open work items linked to the observations discovered in
    // its runs — is refused; `force` overrides, and the response lists
    // what the override rode over. The gate covers BOTH delivery forms
    // (agent run tools and user CLI): it lives in the verb, not in a
    // caller.
    let radius = closure_radius(&state, project, &request.item_key).await?;
    let forced = request.force.unwrap_or(false);
    if !radius.is_empty() && !forced {
        let listing = radius
            .iter()
            .map(|item| format!("{} ({})", item.item_key, item.title))
            .collect::<Vec<_>>()
            .join("; ");
        tracing::warn!(
            item_key = %request.item_key,
            open_radius = radius.len(),
            "close refused: the closure radius is open (ADR-0038)"
        );
        return Err(ApiError::with_code(
            StatusCode::CONFLICT,
            format!(
                "item {} has an open closure radius — {} open item(s) are linked to the \
                 observations of its runs: {listing}. Close them first, or retry the close \
                 with force: true to override with this reason",
                request.item_key,
                radius.len()
            ),
            "close_radius_open",
        ));
    }
    if !radius.is_empty() && forced {
        // The override is the HUMAN surface (rust-security F1, sprint-70
        // gate): the tool client pins force=false for agents, and the
        // verb refuses the worker principal itself — a run posting
        // {"force": true} directly never bypasses the guardrail the gate
        // exists to make accountable.
        refuse_worker(&auth, "force a close over an open closure radius")?;
    }
    let tracker = tracker_for(&state, project).await?;
    tracker
        .close(project, &request.item_key, &request.reason)
        .await
        .map_err(tracker_failed)?;
    if let Some(claim) = adjudicator_claim {
        // The adjudicator's close is merge-complete (stage E): the claim
        // LANDS — the supervisor's land path, moved into the door when
        // its merge arms retired. A failed land fails the close loudly;
        // the tracker item is closed, the claim heals through the
        // reconciler's land retry.
        let landed = state
            .work_store()
            .land(&claim.claim_id, claim.pr_url.as_deref())
            .await
            .map_err(store_error)?;
        best_effort_clear_claim(&state, project, &landed).await;
        if let Err(error) = run_records::append(
            &state,
            claim.run_id,
            PlatformRecord::WorkLanded(WorkLandedRecord {
                claim_id:   landed.claim_id.clone(),
                project_id: landed.project_id.clone(),
                item_key:   landed.item_key.clone(),
            }),
        )
        .await
        {
            tracing::warn!(
                claim_id = %landed.claim_id,
                run_id = %claim.run_id,
                error = ?error,
                "work item close: land record append failed"
            );
        }
    } else {
        // A closed item releases any live claim (DB first, projection
        // second).
        if let Some(claim) = state
            .work_store()
            .active_claim_for_item(&project.project_id, &request.item_key)
            .await
            .map_err(store_error)?
        {
            state
                .work_store()
                .release(&claim.claim_id, "item closed")
                .await
                .map_err(store_error)?;
            best_effort_clear_claim(&state, project, &claim).await;
        }
    }
    let item = tracker
        .get(project, &request.item_key)
        .await
        .map_err(tracker_failed)?
        .ok_or_else(|| not_found("work item", &request.item_key))?;
    if !radius.is_empty() {
        tracing::warn!(
            item_key = %request.item_key,
            overridden = radius.len(),
            "close forced over an open closure radius (ADR-0038); the response lists the items"
        );
    }
    Ok(Json(WorkItemCloseResponse {
        item:              api_item(item),
        overridden_radius: if forced { radius } else { Vec::new() },
    })
    .into_response())
}

/// The closure radius as its own read (ADR-0039 stage D): the SAME
/// computation the close gate enforces, served for the adjudicator's
/// fast path (the trivial case demands an empty radius) and the
/// operator. One implementation, two readers.
async fn get_work_item_radius(
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path((project_id, item_key)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let caller = resolve_caller_project(&state, &auth, &project_id).await?;
    let project = caller.project();
    let tracker = tracker_for(&state, project).await?;
    let item = tracker
        .get(project, &item_key)
        .await
        .map_err(tracker_failed)?
        .ok_or_else(|| ApiError::not_found("Work item not found."))?;
    let radius = closure_radius(&state, project, &item_key).await?;
    Ok(Json(WorkItemRadiusResponse {
        item: api_item(item),
        radius,
    })
    .into_response())
}

/// The comment READ verb (ADR-0039 stage D part 2, denkhaus/work#1107):
/// a work item's comments, oldest first — the ANSWER CHANNEL for parked
/// runs. The adjudicator parks a policy fork as a `parked-fork:` note on
/// the run; the operator answers on the work item; the next pass reads
/// the answer here. Same authorization and item-existence gate as the
/// radius read: the project's workers may read, comment writing stays
/// the human's surface (tracker/UI).
async fn list_work_item_comments(
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path((project_id, item_key)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let caller = resolve_caller_project(&state, &auth, &project_id).await?;
    let project = caller.project();
    let tracker = tracker_for(&state, project).await?;
    tracker
        .get(project, &item_key)
        .await
        .map_err(tracker_failed)?
        .ok_or_else(|| ApiError::not_found("Work item not found."))?;
    let comments = tracker
        .list_comments(project, &item_key)
        .await
        .map_err(tracker_failed)?;
    // `WorkItemComment` IS the port's type (the `with_replacement`
    // contract, AGENTS.md API type ownership): no adapter, the wire
    // shape is the canonical shape.
    Ok(Json(WorkItemCommentList { comments }).into_response())
}

/// The closure radius of one work item (ADR-0038): the OPEN work items
/// linked to the observation records discovered in the item's runs —
/// item → runs (the whole claim history: the claim store is the
/// authority, WorkClaimed records are audit) → their
/// `observation.recorded` records → the links' items, minus the item
/// itself (an item linked to its own run's observations is a self-loop,
/// never a self-block). One level: "transitive" names the multi-edge
/// chain through runs and observations, never recursion into the radius
/// items' own radii.
///
/// The observation read degrades to empty on a store failure (with a
/// warn): the radius must never wedge closes on a transient read error —
/// the same degrade-to-close philosophy the closeout's advisory arms
/// use. The links themselves live in the work store beside the claims;
/// a failure there propagates (it is the same database the claim check
/// just read).
async fn closure_radius(
    state: &Arc<AppState>,
    project: &WorkProject,
    item_key: &str,
) -> Result<Vec<ApiWorkItem>, ApiError> {
    let runs = state
        .work_store()
        .runs_for_item(&project.project_id, item_key)
        .await
        .map_err(store_error)?;
    let mut observations = Vec::new();
    for run_id in &runs {
        match state
            .stores
            .run_summaries
            .platform_records()
            .read_kind(run_id, PlatformRecordKind::ObservationRecorded)
            .await
        {
            Ok(records) => {
                observations.extend(records.iter().map(|stored| (*run_id, stored.seq)));
            }
            Err(error) => {
                tracing::warn!(
                    run_id = %run_id,
                    error = %error,
                    "observation record read failed; the radius degrades to the links it can see"
                );
            }
        }
    }
    let links = state
        .work_store()
        .linked_items_for_observations(&project.project_id, &observations)
        .await
        .map_err(store_error)?;
    let mut radius_keys: Vec<String> = links
        .into_iter()
        .map(|link| link.item_key)
        .filter(|key| key != item_key)
        .collect();
    radius_keys.sort_unstable();
    radius_keys.dedup();
    if radius_keys.is_empty() {
        return Ok(Vec::new());
    }
    let tracker = tracker_for(state, project).await?;
    let mut radius = Vec::new();
    for key in &radius_keys {
        if let Some(item) = tracker.get(project, key).await.map_err(tracker_failed)? {
            if item.state == "open" {
                radius.push(api_item(item));
            }
        }
    }
    Ok(radius)
}

/// Link a work item to an observation record (ADR-0038): the user-only
/// backfill/bench surface. Worker principals are refused — the same
/// posture as the lane move; the run-end hook writes links internally
/// server-side when it lands.
async fn link_observation(
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(request): Json<WorkObservationLinkRequest>,
) -> Result<Response, ApiError> {
    refuse_worker(&auth, "write observation links")?;
    let project = resolve_project(&state, &auth, &project_id).await?;
    let run_id = parse_link_run_id(&request.run_id)?;
    let seq = u64::from(request.seq);
    let outcome = state
        .work_store()
        .link_observation(&project.project_id, &request.item_key, &run_id, seq)
        .await
        .map_err(store_error)?;
    // The write door's materialization (work#1385): a NEW link changes
    // the record's derived state — re-derive it once and store it beside
    // the record. An already-linked pair changed nothing; the state the
    // first write materialized stands.
    if outcome == fabro_work::LinkOutcome::Linked {
        super::observation_triage::rederive_and_materialize(&state, &run_id, seq).await;
    }
    Ok(Json(WorkObservationLinkResponse {
        outcome:  match outcome {
            fabro_work::LinkOutcome::Linked => WorkObservationLinkOutcome::Linked,
            fabro_work::LinkOutcome::AlreadyLinked => WorkObservationLinkOutcome::AlreadyLinked,
        },
        item_key: request.item_key,
        run_id:   request.run_id,
        seq:      i64::try_from(seq).unwrap_or(i64::MAX),
    })
    .into_response())
}

/// Remove one observation link — user-only, like link. Unlinking an
/// absent pair is `removed: false`, never an error.
async fn unlink_observation(
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(request): Json<WorkObservationLinkRequest>,
) -> Result<Response, ApiError> {
    refuse_worker(&auth, "write observation links")?;
    let project = resolve_project(&state, &auth, &project_id).await?;
    let run_id = parse_link_run_id(&request.run_id)?;
    let removed = state
        .work_store()
        .unlink_observation(
            &project.project_id,
            &request.item_key,
            &run_id,
            u64::from(request.seq),
        )
        .await
        .map_err(store_error)?;
    // The write door's materialization (work#1385): the record may have
    // lost its last link — its derived state re-derives from whatever
    // links and decision remain. Removing an absent pair changed
    // nothing.
    if removed {
        super::observation_triage::rederive_and_materialize(
            &state,
            &run_id,
            u64::from(request.seq),
        )
        .await;
    }
    Ok(Json(WorkObservationUnlinkResponse { removed }).into_response())
}

/// The link surface is a USER operation: a worker principal is refused
/// before any store call (the lane-move posture, denkhaus/work#1065).
fn refuse_worker(auth: &RequiredRunManagementActor, action: &str) -> Result<(), ApiError> {
    if let fabro_types::Principal::Worker { run_id } = &auth.0 {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            format!("observation links are a user operation — run {run_id} may not {action}"),
        ));
    }
    Ok(())
}

fn parse_link_run_id(run_id: &str) -> Result<RunId, ApiError> {
    run_id.parse::<RunId>().map_err(|err| {
        ApiError::bad_request(format!("invalid runId for the observation link: {err}"))
    })
}

async fn update_work_item(
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(request): Json<WorkItemUpdateRequest>,
) -> Result<Response, ApiError> {
    if request.title.as_deref().map_or("", str::trim).is_empty()
        && request.body.as_deref().map_or("", str::trim).is_empty()
    {
        return Err(ApiError::bad_request(
            "work item update needs a title or a body",
        ));
    }
    let caller = resolve_caller_project(&state, &auth, &project_id).await?;
    let project = caller.project();
    // Workers edit only items they hold a live claim on — spec
    // corrections happen while the run works the item
    // (denkhaus/work#991).
    require_claim_held_by_run(&state, &caller, &request.item_key).await?;
    let tracker = tracker_for(&state, project).await?;
    let title = request
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let body = request
        .body
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let item = tracker
        .update(project, &request.item_key, title, body)
        .await
        .map_err(tracker_failed)?;
    Ok(Json(api_item(item)).into_response())
}

/// The lane move is a USER operation (denkhaus/work#1065): the tranche
/// model's session move, never an agent-run capability — a worker
/// principal is refused before any store or tracker call, so the run
/// tools never carry lane assignment.
async fn lane_work_item(
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(request): Json<WorkItemLaneRequest>,
) -> Result<Response, ApiError> {
    if let fabro_types::Principal::Worker { run_id } = &auth.0 {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            format!("lane assignment is a user operation — run {run_id} may not move lanes"),
        ));
    }
    let project = resolve_project(&state, &auth, &project_id).await?;
    // Same validation posture as `--lane` on create (the CLI's
    // `lane_label`): an unknown lane is refused, naming the declared
    // lanes, so a typo never silently unlans the item.
    if let Some(lane) = request.lane.as_deref() {
        if !project.lane_labels.contains_key(lane) {
            let declared = if project.lane_labels.is_empty() {
                "none declared".to_string()
            } else {
                let mut lanes: Vec<&str> = project.lane_labels.keys().map(String::as_str).collect();
                lanes.sort_unstable();
                lanes.join(", ")
            };
            return Err(ApiError::bad_request(format!(
                "work project {} declares no lane {lane:?} (declared lanes: {declared})",
                project.project_id
            )));
        }
    }
    let tracker = tracker_for(&state, &project).await?;
    let item = tracker
        .set_lane(&project, &request.item_key, request.lane.as_deref())
        .await
        .map_err(tracker_failed)?;
    Ok(Json(api_item(item)).into_response())
}

/// The operator lock door (denkhaus/work#1420): `lock` places the lock,
/// `unlock` lifts it. USER operations (the lane-move posture): the lock
/// is the human surface — a worker principal is refused before any store
/// or tracker call, so the run tools never carry it. The lock rides the
/// store's lock table (never a tracker label), so it survives tracker
/// round-trips; the lane stays untouched by both verbs. The lock's
/// enforcement lives in the claim path: the pool filter (the
/// stale-basis-skip layer) and the grant door's refusal.
async fn lock_work_item(
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(request): Json<WorkItemLockRequest>,
) -> Result<Response, ApiError> {
    if let fabro_types::Principal::Worker { run_id } = &auth.0 {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            format!("the item lock is a user operation — run {run_id} may not lock items"),
        ));
    }
    let project = resolve_project(&state, &auth, &project_id).await?;
    let tracker = tracker_for(&state, &project).await?;
    // The lock names a real item: an unknown key is a 404, not a lock
    // row pointing into the void (the close door's item gate).
    let item = tracker
        .get(&project, &request.item_key)
        .await
        .map_err(tracker_failed)?
        .ok_or_else(|| not_found("work item", &request.item_key))?;
    // Refused while the item carries a live claim (the message names
    // it): the lock is for items no run owns. Idempotent for an
    // already-locked item.
    state
        .work_store()
        .lock_item(&project.project_id, &request.item_key)
        .await
        .map_err(store_error)?;
    tracing::info!(
        project = %project.project_id,
        item_key = %request.item_key,
        "operator lock placed"
    );
    Ok(Json(stamp_lock(api_item(item), true)).into_response())
}

/// The lock door's lift (denkhaus/work#1420): the item becomes grantable
/// again; the lane is untouched. Unlocking an unlocked item succeeds —
/// the response just reports the item unlocked.
async fn unlock_work_item(
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(request): Json<WorkItemLockRequest>,
) -> Result<Response, ApiError> {
    if let fabro_types::Principal::Worker { run_id } = &auth.0 {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            format!("the item lock is a user operation — run {run_id} may not unlock items"),
        ));
    }
    let project = resolve_project(&state, &auth, &project_id).await?;
    let tracker = tracker_for(&state, &project).await?;
    let item = tracker
        .get(&project, &request.item_key)
        .await
        .map_err(tracker_failed)?
        .ok_or_else(|| not_found("work item", &request.item_key))?;
    let removed = state
        .work_store()
        .unlock_item(&project.project_id, &request.item_key)
        .await
        .map_err(store_error)?;
    tracing::info!(
        project = %project.project_id,
        item_key = %request.item_key,
        removed,
        "operator lock lifted"
    );
    Ok(Json(stamp_lock(api_item(item), false)).into_response())
}

/// Stamps the operator-lock badge (denkhaus/work#1420) onto a served
/// item: the lock lives in the store, not the tracker, so the
/// tracker-shaped item cannot carry it and the door overlays it for the
/// wire. `locked` is optional in the schema — the doors that stamp it
/// (list, the single-item reads, lock/unlock) answer truthfully; a door
/// that leaves it unset makes no claim either way.
fn stamp_lock(item: ApiWorkItem, locked: bool) -> ApiWorkItem {
    ApiWorkItem {
        locked: Some(locked),
        ..item
    }
}

async fn claim_next(
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    Json(request): Json<WorkClaimNextRequest>,
) -> Result<Response, ApiError> {
    let run_id = request
        .run_id
        .parse::<RunId>()
        .map_err(|err| ApiError::bad_request(format!("invalid runId: {err}")))?;
    let caller = resolve_caller_project(&state, &auth, &project_id).await?;
    // Workers claim next in their own project for their OWN run only
    // (denkhaus/work#991): a foreign body runId is refused loudly.
    let run_id = bind_granted_run_id(&caller, run_id)?;
    let project = caller.project();
    let tracker = state
        .work_tracker_for_project(project)
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("tracker provider is not available: {error:#}"),
            )
        })?;
    let items = tracker
        .list(project, WorkItemFilter {
            lane:  Some(request.lane.clone()),
            limit: 50,
            query: None,
            label: None,
            page:  1,
            state: WorkItemStateFilter::Open,
        })
        .await
        .map_err(tracker_failed)?;
    let live_claims: std::collections::HashMap<String, fabro_work::WorkClaim> = state
        .work_store()
        .active_claims(&project.project_id)
        .await
        .map_err(store_error)?
        .into_iter()
        .map(|claim| (claim.item_key.clone(), claim))
        .collect();
    let mut claimed: std::collections::HashSet<String> = live_claims.keys().cloned().collect();

    // Misroute skip (denkhaus/work#1179): a run that released an item —
    // misroute, stale basis, planner skip — is never re-granted the same
    // item for the rest of the run's lifetime (denkhaus/work#1186); a
    // windowed skip expired under runs that outlive the window and
    // re-served every released item in a release-and-ask livelock.
    // Without the skip the lane head cycles over items no run of the
    // lane can land and never surfaces the lower-priority landable
    // candidates; with it, a run that declined every open lane item
    // gets an empty claim. The skip is per run; another run stays free
    // to claim the skipped item.
    let released_by_run = state
        .work_store()
        .released_claims_of_run(&run_id)
        .await
        .map_err(store_error)?;
    claimed.extend(released_by_run.into_iter().map(|claim| claim.item_key));

    // Durable stale-basis skip (denkhaus/work#1377): an item whose
    // LATEST release is a stale-basis verdict stays out of the pool —
    // the per-run skip above deliberately does not protect other runs
    // (#1186), so the next lap would otherwise re-offer, re-verify, and
    // re-burn a full run on a landed-but-open item until the human
    // triage closes it. The verdict expires when the ITEM moves past it
    // (any touch bumps `updated_at`; an unknown timestamp counts as
    // unmoved — suppressing a possibly actionable item is the cheap
    // error, re-burning a run is the expensive one) and dies with the
    // item's closure. The human triage close stays the only way the
    // item leaves the tracker.
    let stale_basis = state
        .work_store()
        .stale_basis_verdicts(&project.project_id)
        .await
        .map_err(store_error)?;

    // Operator lock door (denkhaus/work#1420): an item carrying an
    // operator-held lock is not grantable — regardless of lane, with no
    // expiry (the operator lifts it). The same pool layer as the
    // stale-basis skip above; the grant door's own refusal (the store's
    // `grant`) is the second net. The lane is untouched by the lock, so
    // the item still lists under its lane for re-routing decisions.
    let locked_items: std::collections::HashSet<String> = state
        .work_store()
        .locked_items(&project.project_id)
        .await
        .map_err(store_error)?
        .into_iter()
        .collect();

    // Lane backpressure (fabro-4580, ADR-0032): a lane grants only when its
    // open run PRs are drained. Cap 0 = refuse while ANY claim of the lane
    // awaits merge; the scheduler cannot fire a second overlapping run past
    // the grant even when it forgets the guard.
    let awaiting_merge = state
        .work_store()
        .active_claims_of_lane(&project.project_id, &request.lane)
        .await
        .map_err(store_error)?
        .into_iter()
        .filter(|claim| claim.state == ClaimState::AwaitingMerge)
        .count();
    if awaiting_merge > usize::try_from(project.backpressure_open_pr_cap.max(0)).unwrap_or(0) {
        let blocking = state
            .work_store()
            .active_claims_of_lane(&project.project_id, &request.lane)
            .await
            .map_err(store_error)?
            .into_iter()
            .find(|claim| claim.state == ClaimState::AwaitingMerge);
        let blocking_item = match blocking.as_ref() {
            Some(claim) => tracker.get(project, &claim.item_key).await.ok().flatten(),
            None => None,
        };
        return Ok(Json(WorkClaimNextResponse {
            outcome: WorkClaimNextOutcome::Blocked,
            claim:   blocking.map(api_claim).transpose()?,
            item:    blocking_item.map(api_item),
        })
        .into_response());
    }

    for item in items {
        if locked_items.contains(&item.item_key) {
            tracing::debug!(
                item_key = %item.item_key,
                "work pool: candidate item carries an operator lock; skipped (work#1420)"
            );
            continue;
        }
        if claimed.contains(&item.item_key) {
            // The claim-hold arm of the pool policy (denkhaus/work#1457,
            // ADR-0045 P1): a candidate whose live claim HOLDS stays out
            // of the pool — the park pin included (work#1419,
            // work#1448): a parked `human_needed` run's claim yields to
            // nothing but its operator's disposition, never to a fresh
            // lap's grant (the production burn on denkhaus/work#1312).
            // A live claim whose hold has lapsed (`hold_reason` answers
            // None — a refused verdict pending the reconciler's heal)
            // falls through to the grant attempt: the ledger stays the
            // arbiter (ADR-0032 decision 2) and the unique index refuses
            // a true collision. Released-by-run entries in `claimed` are
            // the misroute skip (#1179/#1186) — no hold to consult.
            if let Some(claim) = live_claims.get(&item.item_key) {
                if live_claim_holds(&state, claim).await {
                    continue;
                }
            } else {
                continue;
            }
        }
        if let Some(released_at) = stale_basis.get(&item.item_key) {
            let moved_past = item
                .updated_at
                .is_some_and(|updated| updated > *released_at);
            if !moved_past {
                continue;
            }
        }
        match state
            .work_store()
            .grant(&GrantRequest {
                project_id: project.project_id.clone(),
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
                    .project_claim(project, &item.item_key, &claim.run_id.to_string())
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
                    .get(project, &claim.item_key)
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
        let held_item = tracker.get(project, &claim.item_key).await.ok().flatten();
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

/// The pool policy's hold classification for one live claim
/// (denkhaus/work#1457): fetches the owning run's status and its latest
/// merge verdict, then asks the claim-hold seam (`fabro_work::hold_reason`).
/// `true` = the claim holds and the item stays out of the pool. A run the
/// store cannot serve counts as holding: the reconciler's orphan grace
/// owns that release, and the pool never races a claim it cannot classify.
async fn live_claim_holds(state: &AppState, claim: &fabro_work::WorkClaim) -> bool {
    let Some(projection) = super::super::run_records::projection(state, claim.run_id)
        .await
        .ok()
        .flatten()
    else {
        tracing::debug!(
            claim_id = %claim.claim_id,
            run_id = %claim.run_id,
            item_key = %claim.item_key,
            "work pool: candidate's claim names a run without a served projection; holding"
        );
        return true;
    };
    let verdict = super::super::work_reconciler::merge_verdict_of_run(state, claim.run_id).await;
    match fabro_work::hold_reason(claim, projection.status, verdict) {
        Some(reason) => {
            tracing::debug!(
                claim_id = %claim.claim_id,
                run_id = %claim.run_id,
                item_key = %claim.item_key,
                hold = %reason,
                "work pool: candidate item is claimed and holds"
            );
            true
        }
        None => false,
    }
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
    auth: RequiredRunManagementActor,
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
    ExtraQuery(params): ExtraQuery<WorkItemKeyQuery>,
) -> Result<Response, ApiError> {
    let item_key = params
        .item_key
        .ok_or_else(|| ApiError::bad_request("itemKey is required"))?;
    // Workers read their own project's items only (denkhaus/work#991).
    let caller = resolve_caller_project(&state, &auth, &project_id).await?;
    let project = caller.project();
    let tracker = tracker_for(&state, project).await?;
    let item = tracker
        .get(project, &item_key)
        .await
        .map_err(tracker_failed)?
        .ok_or_else(|| not_found("work item", &item_key))?;
    let locked = state
        .work_store()
        .item_lock(&project.project_id, &item_key)
        .await
        .map_err(store_error)?
        .is_some();
    Ok(Json(stamp_lock(api_item(item), locked)).into_response())
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

/// `self` resolves to the project bound to the authenticated run's target
/// repository (project-agnostic workflows, ADR-0032 phase 3): the prompts
/// name neither vendor, repository nor project id — the run-scoped caller
/// says `self` and the server owns the binding. A non-run caller (a user)
/// has no `self`; every literal id takes the direct path.
async fn resolve_project(
    state: &Arc<AppState>,
    auth: &RequiredRunManagementActor,
    project_id: &str,
) -> Result<WorkProject, ApiError> {
    if project_id != "self" {
        return load_project(state, project_id).await;
    }
    let fabro_types::Principal::Worker { run_id } = &auth.0 else {
        return Err(ApiError::bad_request(
            "project `self` resolves through a run-scoped caller; a user names the project id",
        ));
    };
    run_own_project(state, run_id).await
}

/// The run→project seam behind `self`: a run's target repository names
/// the one work project its worker may touch (denkhaus/work#991).
async fn run_own_project(state: &Arc<AppState>, run_id: &RunId) -> Result<WorkProject, ApiError> {
    let projection = state.load_run_projection(run_id).await.map_err(|_| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            format!("run {run_id} not found — cannot resolve its work project"),
        )
    })?;
    let Some(fabro_types::RunTarget::Git(target)) = projection.spec.target.as_ref() else {
        return Err(ApiError::bad_request(
            "run has no git target — its work project cannot resolve; name the project id",
        ));
    };
    state
        .work_store()
        .project_for_target(&target.repo)
        .await
        .map_err(store_error)?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::NOT_FOUND,
                format!(
                    "no work project is bound to target repository {} — configure the project's \
                     targetRepo or name the project id",
                    target.repo
                ),
            )
        })
}

/// The work-route caller with its resolved project binding
/// (denkhaus/work#991, grill 2026-10-26): a user may name any project
/// and touch any claim (the human surface); the run's worker is BOUND
/// to the project its run's target repository names — reads and writes
/// alike (ADR-0019 least privilege), one policy for both delivery
/// forms: the agent run tools travel these routes with the same
/// principal, so hardening here binds them in the same breath.
#[derive(Debug)]
enum WorkCaller {
    /// A user: any project, any claim.
    User { project: WorkProject },
    /// The run's worker: the run's own project only.
    RunWorker {
        run_id:  RunId,
        project: WorkProject,
    },
}

impl WorkCaller {
    fn project(&self) -> &WorkProject {
        match self {
            Self::User { project } | Self::RunWorker { project, .. } => project,
        }
    }
}

/// Resolves the path's project id into a [`WorkCaller`]: users take the
/// direct (or `self`-refusing) path; workers resolve their own project
/// and every OTHER literal id — foreign reads included — is refused
/// with the run's scope in the message.
async fn resolve_caller_project(
    state: &Arc<AppState>,
    auth: &RequiredRunManagementActor,
    project_id: &str,
) -> Result<WorkCaller, ApiError> {
    match &auth.0 {
        fabro_types::Principal::User(_) => Ok(WorkCaller::User {
            project: resolve_project(state, auth, project_id).await?,
        }),
        fabro_types::Principal::Worker { run_id } => {
            let project = run_own_project(state, run_id).await?;
            if project_id == "self" || project_id == project.project_id {
                Ok(WorkCaller::RunWorker {
                    run_id: *run_id,
                    project,
                })
            } else {
                Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    format!(
                        "run {run_id} is scoped to work project {} — {project_id} is beyond its \
                         scope",
                        project.project_id
                    ),
                ))
            }
        }
        // RequiredRunManagementActor passes only users and scoped workers.
        _ => Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "work routes serve users and run workers only",
        )),
    }
}

/// One policy for claim-owning routes: a user may touch any claim; the
/// run's worker only claims whose `run_id` is its own (denkhaus/work#991).
fn bind_claim_to_caller(
    auth: &RequiredRunManagementActor,
    claim: &WorkClaim,
) -> Result<(), ApiError> {
    if let fabro_types::Principal::Worker { run_id } = &auth.0 {
        if claim.run_id != *run_id {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                // Kind-only (work#1043 finding 4): the refusal names
                // the condition — no run ids, no claim ids — so a
                // sibling run learns nothing enumerable about who holds
                // the claim (error-handling strategy: curated public
                // messages).
                "this claim is held by another run; a run may only touch its own claims"
                    .to_string(),
            ));
        }
    }
    Ok(())
}

/// Loads the claim AND binds it to the caller in one step — the
/// transition/release twins share exactly this shape (code-review
/// sprint 55+56 finding: one helper instead of two inline copies), and
/// the binding runs BEFORE any store mutation.
async fn load_bound_claim(
    state: &Arc<AppState>,
    auth: &RequiredRunManagementActor,
    claim_id: &str,
) -> Result<WorkClaim, ApiError> {
    validate_claim_id(claim_id)?;
    let claim = state
        .work_store()
        .claim(claim_id)
        .await
        .map_err(store_error)?
        .ok_or_else(|| not_found("work claim", claim_id))?;
    bind_claim_to_caller(auth, &claim)?;
    Ok(claim)
}

/// `grant`/`next` are pinned to the authenticated run for workers
/// (denkhaus/work#991): a body runId naming a foreign run is refused
/// loudly instead of silently stamped — a caller bug must surface, not
/// land as a grant to the wrong run. Users keep granting for any run.
fn bind_granted_run_id(caller: &WorkCaller, request_run_id: RunId) -> Result<RunId, ApiError> {
    match caller {
        WorkCaller::User { .. } => Ok(request_run_id),
        WorkCaller::RunWorker { run_id, .. } => {
            if request_run_id == *run_id {
                Ok(request_run_id)
            } else {
                Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    format!(
                        "grant is pinned to the authenticated run {run_id} — body runId \
                         {request_run_id} names a different run",
                    ),
                ))
            }
        }
    }
}

/// Close and update are claim-bound for workers (denkhaus/work#991): the
/// run must HOLD a live claim on the item — closing or editing an item
/// another run works, or one nobody holds, is beyond the run's scope.
async fn require_claim_held_by_run(
    state: &Arc<AppState>,
    caller: &WorkCaller,
    item_key: &str,
) -> Result<(), ApiError> {
    let WorkCaller::RunWorker { run_id, project } = caller else {
        return Ok(());
    };
    let held = state
        .work_store()
        .active_claim_for_item(&project.project_id, item_key)
        .await
        .map_err(store_error)?;
    match held {
        Some(claim) if claim.run_id == *run_id => Ok(()),
        Some(claim) => Err(ApiError::new(
            StatusCode::FORBIDDEN,
            format!(
                "item {item_key} is claimed by run {} — run {run_id} may only close or edit its \
                 own claimed items",
                claim.run_id
            ),
        )),
        None => Err(ApiError::new(
            StatusCode::FORBIDDEN,
            format!(
                "item {item_key} has no live claim — run {run_id} may only close or edit items \
                 it holds a claim on",
            ),
        )),
    }
}

/// Resolve the worker-close authority (ADR-0039 stage E). `Ok(None)` is
/// the user path (unchanged). A worker must run the `adjudicator`
/// workflow AND the item's claim must belong to a run the merger
/// enabled (`closable: enabled`) — the returned claim is the one the
/// close will LAND. Every other worker shape refuses, teaching the
/// rule it broke.
async fn adjudicator_close_authority(
    state: &Arc<AppState>,
    auth: &RequiredRunManagementActor,
    project: &WorkProject,
    item_key: &str,
) -> Result<Option<WorkClaim>, ApiError> {
    let fabro_types::Principal::Worker { run_id } = &auth.0 else {
        return Ok(None);
    };
    let summary = state
        .stores
        .run_summaries
        .get(run_id, chrono::Utc::now())
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("reading the caller's run summary failed: {error}"),
            )
        })?
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::FORBIDDEN,
                "the worker run named by the token does not exist",
            )
        })?;
    if summary.workflow.slug.as_deref() != Some(super::orchestration::MERGE_WRITER_WORKFLOW) {
        return Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            "only the adjudicator closes work items — a run never closes its own item; the \
             adjudicator's closing pass closes it once the merger enabled closable \
             (ADR-0039 decisions 1 and 6)",
            "close_not_adjudicator",
        ));
    }
    let claim = state
        .work_store()
        .active_claim_for_item(&project.project_id, item_key)
        .await
        .map_err(store_error)?
        .ok_or_else(|| {
            ApiError::with_code(
                StatusCode::FORBIDDEN,
                format!("item {item_key} has no live claim — there is nothing to land"),
                "close_no_live_claim",
            )
        })?;
    // The verdict the close rides on: the CLAIM RUN's closable verdict,
    // not the caller's. One read of the field's fold.
    let verdict = state
        .stores
        .run_summaries
        .platform_records()
        .latest_verdict_of_run(fabro_types::OrchestrationField::Closable, &claim.run_id)
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("reading the closable verdicts failed: {error}"),
            )
        })?;
    match verdict {
        Some(fabro_types::OrchestrationVerdict::Enabled) => Ok(Some(claim)),
        verdict => Err(ApiError::with_code(
            StatusCode::FORBIDDEN,
            format!(
                "item {item_key} waits for `closable: enabled` on its run — the close is \
                 merge-complete, and the merger's latest verdict is {} (ADR-0039 decision 6)",
                verdict.map_or_else(
                    || "none (never adjudicated)".to_string(),
                    |verdict| verdict.to_string()
                )
            ),
            "close_not_closable",
        )),
    }
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
        // The lock badge (denkhaus/work#1420) is a store overlay, not
        // tracker state: unset here, stamped by the doors that read the
        // lock table (stamp_lock).
        locked:     None,
        priority:   item.priority,
        state:      item.state,
        title:      item.title,
        updated_at: item.updated_at,
        url:        item.url,
        created_at: item.created_at,
        closed_at:  item.closed_at,
    }
}

fn api_project(project: WorkProject) -> ApiWorkProject {
    ApiWorkProject {
        backpressure_open_pr_cap: project.backpressure_open_pr_cap,
        claim_comment_enabled:    project.claim_comment_enabled,
        claim_label_prefix:       project.claim_label_prefix,
        display_name:             project.display_name,
        lane_labels:              project.lane_labels.into_iter().collect(),
        project_id:               project.project_id,
        project_label:            project.project_label,
        target_repo:              project.target_repo,
        tracker_provider:         project.tracker_provider,
        tracker_repo:             project.tracker_repo,
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

#[cfg(test)]
mod resolve_self_tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use axum::extract::{Path, State};
    use axum::http::{HeaderMap, StatusCode};
    use chrono::Utc;
    use fabro_api::types::WorkClaimNextRequest;
    use fabro_static::EnvVars;
    use fabro_types::{GitRunTarget, Principal, RunId, RunTarget, SystemActorKind, WorkflowPath};
    use fabro_work::{TrackerProvider, WorkItem, WorkItemFilter};

    use super::{
        WorkCaller, bind_claim_to_caller, bind_granted_run_id, require_claim_held_by_run,
        resolve_caller_project, resolve_project,
    };
    use crate::principal_middleware::RequiredRunManagementActor;
    use crate::server::AppState;
    use crate::test_support::TestAppStateBuilder;

    const MINIMAL_DOT: &str = r#"digraph Test {
        graph [goal="Test"]
        start [shape=Mdiamond]
        exit  [shape=Msquare]
        start -> exit
    }"#;

    fn state() -> Arc<AppState> {
        TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
            .build()
    }

    async fn create_run_with_target(state: &Arc<AppState>, target: RunTarget) -> RunId {
        let entrypoint = WorkflowPath::new("workflow.fabro").unwrap();
        let files = BTreeMap::from([(entrypoint.clone(), MINIMAL_DOT.to_string())]);
        let version =
            fabro_types::WorkflowVersion::new(entrypoint, files, BTreeMap::new()).unwrap();
        let version = fabro_workflow_version::ValidatedWorkflowVersion::new(version).unwrap();
        let workflow_version_id =
            fabro_workflow_version::WorkflowVersionStore::new(state.store_ref().blobs())
                .put(&version)
                .await
                .unwrap();
        let run_id = RunId::new();
        let response = Box::pin(crate::server::handler::runs::create_run_from_intent(
            Arc::clone(state),
            crate::server::handler::runs::CreateRunFromIntentRequest {
                intent:          fabro_api::types::RunIntent {
                    workflow_version_id,
                    target,
                    args: fabro_api::types::RunIntentArgs::default(),
                    environment_id: None,
                    parent_id: None,
                    title: None,
                    goal: None,
                    force: None,
                },
                explicit_run_id: Some(run_id),
                actor:           Principal::System {
                    system_kind: SystemActorKind::Engine,
                },
                headers:         HeaderMap::new(),
                automation:      None,
            },
        ))
        .await;
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "the fixture run must be created"
        );
        run_id
    }

    fn bound_project() -> fabro_work::WorkProject {
        fabro_work::WorkProject {
            project_id:               "fabro".to_string(),
            display_name:             "Fabro".to_string(),
            tracker_provider:         "github".to_string(),
            tracker_repo:             "denkhaus/work".to_string(),
            target_repo:              Some("denkhaus/fabro".to_string()),
            project_label:            "project:fabro".to_string(),
            lane_labels:              [("loop".to_string(), "lane:loop".to_string())]
                .into_iter()
                .collect(),
            claim_label_prefix:       "fabro:run:".to_string(),
            claim_comment_enabled:    false,
            backpressure_open_pr_cap: 0,
        }
    }

    #[tokio::test]
    async fn self_resolves_to_the_project_bound_to_the_runs_target_repo() {
        let state = state();
        state
            .work_store()
            .upsert_project(&bound_project())
            .await
            .unwrap();
        let run_id = create_run_with_target(
            &state,
            RunTarget::Git(GitRunTarget {
                repo:   "denkhaus/fabro".to_string(),
                branch: "denkhaus".to_string(),
                tag:    None,
                sha:    None,
            }),
        )
        .await;
        let auth = RequiredRunManagementActor(Principal::Worker { run_id });
        let project = resolve_project(&state, &auth, "self").await.unwrap();
        assert_eq!(project.project_id, "fabro");
        assert_eq!(project.target_repo.as_deref(), Some("denkhaus/fabro"));
    }

    #[tokio::test]
    async fn self_with_an_unbound_target_repo_is_not_found() {
        let state = state();
        state
            .work_store()
            .upsert_project(&bound_project())
            .await
            .unwrap();
        let run_id = create_run_with_target(
            &state,
            RunTarget::Git(GitRunTarget {
                repo:   "denkhaus/other".to_string(),
                branch: "main".to_string(),
                tag:    None,
                sha:    None,
            }),
        )
        .await;
        let auth = RequiredRunManagementActor(Principal::Worker { run_id });
        let error = resolve_project(&state, &auth, "self")
            .await
            .expect_err("no project is bound to the run's target repo");
        assert_eq!(error.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn self_without_a_git_target_is_refused() {
        let state = state();
        let run_id = create_run_with_target(&state, RunTarget::None {}).await;
        let auth = RequiredRunManagementActor(Principal::Worker { run_id });
        let error = resolve_project(&state, &auth, "self")
            .await
            .expect_err("a run without a git target has no `self` project");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn self_flows_through_next_and_lands_in_the_bound_projects_ledger() {
        use fabro_api::types::WorkClaimNextRequest;
        use fabro_work::TrackerProvider;

        use super::claim_next;

        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
            .work_tracker(Arc::new(OneItemTracker) as Arc<dyn TrackerProvider>)
            .build();
        state
            .work_store()
            .upsert_project(&bound_project())
            .await
            .unwrap();
        let run_id = create_run_with_target(
            &state,
            RunTarget::Git(GitRunTarget {
                repo:   "denkhaus/fabro".to_string(),
                branch: "denkhaus".to_string(),
                tag:    None,
                sha:    None,
            }),
        )
        .await;
        let auth = RequiredRunManagementActor(Principal::Worker { run_id });

        let response = claim_next(
            auth,
            State(Arc::clone(&state)),
            Path("self".to_string()),
            axum::Json(WorkClaimNextRequest {
                run_id: run_id.to_string(),
                lane:   "loop".to_string(),
            }),
        )
        .await
        .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["outcome"], "granted", "the run claims through `self`");

        // The claim must live under the RESOLVED project, never a literal
        // `self` row (the ledger split the raw path id used to cause).
        let claims = state.work_store().active_claims("fabro").await.unwrap();
        assert_eq!(
            claims.len(),
            1,
            "the grant lands in the bound project's ledger"
        );
        assert_eq!(claims[0].item_key, "denkhaus/work#1");
        let ghost = state.work_store().active_claims("self").await.unwrap();
        assert!(
            ghost.is_empty(),
            "no claim may land under the literal `self`"
        );
    }

    /// THE LOCK DOOR AT THE POOL (denkhaus/work#1420): the lane's sole
    /// open candidate carries an operator lock — claim_next answers
    /// EMPTY, no claim row lands, and the lane label is untouched. The
    /// lane stays visible for re-routing; the lock is the only hold.
    #[tokio::test]
    async fn an_operator_locked_item_is_never_granted_by_claim_next() {
        use fabro_api::types::WorkClaimNextRequest;
        use fabro_work::TrackerProvider;

        use super::claim_next;

        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
            .work_tracker(Arc::new(OneItemTracker) as Arc<dyn TrackerProvider>)
            .build();
        state
            .work_store()
            .upsert_project(&bound_project())
            .await
            .unwrap();
        // The lock lands only on an unclaimed item — this one.
        state
            .work_store()
            .lock_item("fabro", "denkhaus/work#1")
            .await
            .unwrap();
        let run_id = create_run_with_target(
            &state,
            RunTarget::Git(GitRunTarget {
                repo:   "denkhaus/fabro".to_string(),
                branch: "denkhaus".to_string(),
                tag:    None,
                sha:    None,
            }),
        )
        .await;
        let auth = RequiredRunManagementActor(Principal::Worker { run_id });

        let response = claim_next(
            auth,
            State(Arc::clone(&state)),
            Path("fabro".to_string()),
            axum::Json(WorkClaimNextRequest {
                run_id: run_id.to_string(),
                lane:   "loop".to_string(),
            }),
        )
        .await
        .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            body["outcome"], "empty",
            "the locked candidate is skipped, the pool is empty"
        );
        assert!(
            state
                .work_store()
                .active_claims("fabro")
                .await
                .unwrap()
                .is_empty(),
            "no claim may land for a locked item"
        );
        // The unlock restores grantability — the same run, same pool.
        assert!(
            state
                .work_store()
                .unlock_item("fabro", "denkhaus/work#1")
                .await
                .unwrap(),
            "the unlock lifts a placed lock"
        );
        let auth = RequiredRunManagementActor(Principal::Worker { run_id });
        let response = claim_next(
            auth,
            State(Arc::clone(&state)),
            Path("fabro".to_string()),
            axum::Json(WorkClaimNextRequest {
                run_id: run_id.to_string(),
                lane:   "loop".to_string(),
            }),
        )
        .await
        .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["outcome"], "granted", "the unlocked item grants again");
    }

    /// One open candidate item, static fixture — the shape the original
    /// `claim_next` grant test drives.
    struct OneItemTracker;
    #[async_trait::async_trait]
    impl TrackerProvider for OneItemTracker {
        async fn list(
            &self,
            _project: &fabro_work::WorkProject,
            _filter: WorkItemFilter,
        ) -> anyhow::Result<Vec<WorkItem>> {
            Ok(vec![WorkItem {
                item_key:   "denkhaus/work#1".to_string(),
                title:      "Item 1".to_string(),
                body:       "the spec".to_string(),
                state:      "open".to_string(),
                labels:     vec!["lane:loop".to_string()],
                priority:   1,
                url:        "https://github.com/denkhaus/work/issues/1".to_string(),
                updated_at: Some(Utc::now()),
                created_at: None,
                closed_at:  None,
            }])
        }
        async fn get(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
        ) -> anyhow::Result<Option<WorkItem>> {
            Ok(None)
        }
        async fn create(
            &self,
            _project: &fabro_work::WorkProject,
            _draft: fabro_work::WorkItemDraft,
        ) -> anyhow::Result<WorkItem> {
            anyhow::bail!("not needed")
        }
        async fn update(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
            _title: Option<&str>,
            _body: Option<&str>,
        ) -> anyhow::Result<WorkItem> {
            anyhow::bail!("not needed")
        }
        async fn close(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
            _reason: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn comment(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
            _body: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn project_claim(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
            _run_id: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn clear_claim(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
            _run_id: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    /// One candidate item whose `updated_at` the test can move — the
    /// seam the stale-basis skip's expiry arm (work#1377) reads.
    struct MoveableItemTracker {
        updated_at: std::sync::Mutex<chrono::DateTime<Utc>>,
    }
    #[async_trait::async_trait]
    impl TrackerProvider for MoveableItemTracker {
        async fn list(
            &self,
            _project: &fabro_work::WorkProject,
            _filter: WorkItemFilter,
        ) -> anyhow::Result<Vec<WorkItem>> {
            Ok(vec![WorkItem {
                item_key:   "denkhaus/work#1".to_string(),
                title:      "Item 1".to_string(),
                body:       "the spec".to_string(),
                state:      "open".to_string(),
                labels:     vec!["lane:loop".to_string()],
                priority:   1,
                url:        "https://github.com/denkhaus/work/issues/1".to_string(),
                updated_at: Some(*self.updated_at.lock().unwrap()),
                created_at: None,
                closed_at:  None,
            }])
        }
        async fn get(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
        ) -> anyhow::Result<Option<WorkItem>> {
            Ok(None)
        }
        async fn create(
            &self,
            _project: &fabro_work::WorkProject,
            _draft: fabro_work::WorkItemDraft,
        ) -> anyhow::Result<WorkItem> {
            anyhow::bail!("not needed")
        }
        async fn update(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
            _title: Option<&str>,
            _body: Option<&str>,
        ) -> anyhow::Result<WorkItem> {
            anyhow::bail!("not needed")
        }
        async fn close(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
            _reason: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn comment(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
            _body: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn project_claim(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
            _run_id: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
        async fn clear_claim(
            &self,
            _project: &fabro_work::WorkProject,
            _item_key: &str,
            _run_id: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }
    }

    async fn claim_next_for_run(
        state: &Arc<crate::server::AppState>,
        run_id: RunId,
    ) -> serde_json::Value {
        let response = super::claim_next(
            RequiredRunManagementActor(Principal::Worker { run_id }),
            State(Arc::clone(state)),
            Path("self".to_string()),
            axum::Json(WorkClaimNextRequest {
                run_id: run_id.to_string(),
                lane:   "loop".to_string(),
            }),
        )
        .await
        .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    /// The durable stale-basis skip (denkhaus/work#1377): a released
    /// stale-basis verdict keeps the item out of the NEXT run's claim
    /// pool. The per-run released-claims skip (#1186) deliberately does
    /// not protect other runs, so without this set every
    /// landed-but-open item burned one full run per lap until the human
    /// triage closed it (observed on work#1363, 2026-11-16). The
    /// verdict expires when the item moves past it — and a misroute
    /// release never suppresses (its own test below).
    #[tokio::test]
    async fn a_stale_basis_release_keeps_the_item_out_of_the_next_runs_pool() {
        let tracker = Arc::new(MoveableItemTracker {
            updated_at: std::sync::Mutex::new(Utc::now() - chrono::Duration::hours(1)),
        });
        let bumpable = Arc::clone(&tracker);
        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
            .work_tracker(tracker as Arc<dyn TrackerProvider>)
            .build();
        state
            .work_store()
            .upsert_project(&bound_project())
            .await
            .unwrap();

        let git_target = || {
            RunTarget::Git(GitRunTarget {
                repo:   "denkhaus/fabro".to_string(),
                branch: "denkhaus".to_string(),
                tag:    None,
                sha:    None,
            })
        };

        let first_run = create_run_with_target(&state, git_target()).await;
        let first = claim_next_for_run(&state, first_run).await;
        assert_eq!(
            first["outcome"], "granted",
            "the pool grants the fresh item"
        );
        let claim_id = first["claim"]["claimId"]
            .as_str()
            .expect("the granted claim id")
            .to_string();
        // The planner's stale-basis exit: release with the verdict
        // (the planner prompt's exact words).
        state
            .work_store()
            .release(&claim_id, "stale basis")
            .await
            .unwrap();

        let second_run = create_run_with_target(&state, git_target()).await;
        let second = claim_next_for_run(&state, second_run).await;
        assert_eq!(
            second["outcome"], "empty",
            "the stale-basis verdict suppresses the item for the NEXT run (work#1377): {second}"
        );

        // The item moves past the verdict (any human or system touch
        // bumps updated_at): the pool offers it again.
        *bumpable.updated_at.lock().unwrap() = Utc::now() + chrono::Duration::hours(1);
        let third_run = create_run_with_target(&state, git_target()).await;
        let third = claim_next_for_run(&state, third_run).await;
        assert_eq!(
            third["outcome"], "granted",
            "a touch after the verdict re-offers the item: {third}"
        );
    }

    /// The misroute release is per-run scope only (#1186): it never
    /// suppresses the item for other runs — only the stale-basis
    /// verdict does (work#1377).
    #[tokio::test]
    async fn a_misroute_release_never_suppresses_the_item_for_other_runs() {
        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
            .work_tracker(Arc::new(OneItemTracker) as Arc<dyn TrackerProvider>)
            .build();
        state
            .work_store()
            .upsert_project(&bound_project())
            .await
            .unwrap();

        let git_target = || {
            RunTarget::Git(GitRunTarget {
                repo:   "denkhaus/fabro".to_string(),
                branch: "denkhaus".to_string(),
                tag:    None,
                sha:    None,
            })
        };
        let first_run = create_run_with_target(&state, git_target()).await;
        let first = claim_next_for_run(&state, first_run).await;
        assert_eq!(first["outcome"], "granted");
        let claim_id = first["claim"]["claimId"]
            .as_str()
            .expect("the granted claim id")
            .to_string();
        state
            .work_store()
            .release(&claim_id, "misroute")
            .await
            .unwrap();

        let second_run = create_run_with_target(&state, git_target()).await;
        let second = claim_next_for_run(&state, second_run).await;
        assert_eq!(
            second["outcome"], "granted",
            "a misroute release stays per-run — other runs are offered the item: {second}"
        );
    }

    /// THE HEADLINE PIN (denkhaus/work#1457, ADR-0045 P1; the production
    /// burn on denkhaus/work#1312, runs 01M7NWYS…/01M7QB9Y…): a grant
    /// attempt against a live claim held by a parked `human_needed` run
    /// is REFUSED — the pool's hold arm skips the item, the fresh lap
    /// gets nothing, and the parked run's claim stays live. A parked run
    /// is a WAITING state to the hold seam (ADR-0045): the claim yields
    /// to nothing but its operator's disposition, never to a new grant.
    #[tokio::test]
    async fn a_grant_against_a_live_claim_held_by_a_parked_run_is_refused() {
        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
            .work_tracker(Arc::new(OneItemTracker) as Arc<dyn TrackerProvider>)
            .build();
        state
            .work_store()
            .upsert_project(&bound_project())
            .await
            .unwrap();

        // Lap one: the run claims the lane's only item…
        let parked_run = create_run_with_target(
            &state,
            RunTarget::Git(GitRunTarget {
                repo:   "denkhaus/fabro".to_string(),
                branch: "denkhaus".to_string(),
                tag:    None,
                sha:    None,
            }),
        )
        .await;
        let first = claim_next_for_run(&state, parked_run).await;
        assert_eq!(first["outcome"], "granted", "lap one claims the item");
        let claim_id = first["claim"]["claimId"]
            .as_str()
            .expect("the granted claim id")
            .to_string();

        // …then parks: the decision request (status human_needed,
        // terminal today, a WAITING state to the hold seam).
        let mut projection = fabro_types::RunProjection::new(
            "parked run".to_string(),
            fabro_types::RunSpec {
                run_id:              parked_run,
                settings:            fabro_types::WorkflowSettings::default(),
                graph:               fabro_types::RunGraph::new("test"),
                graph_source:        None,
                workflow_slug:       Some("develop".to_string()),
                workflow_version_id: None,
                target:              None,
                automation:          None,
                source_directory:    None,
                labels:              std::collections::HashMap::new(),
                provenance:          fabro_types::test_support::test_run_provenance(),
                definition_blob:     None,
                spec_blob:           None,
                git:                 None,
                fork_source_ref:     None,
                admission:           fabro_types::PetriAdmission::default(),
            },
            chrono::Utc::now(),
        );
        projection.status = fabro_types::RunStatus::HumanNeeded;
        let pool = state.stores.run_summaries.pool();
        let mut tx = pool.begin().await.expect("seed tx");
        fabro_store::RunSummaryStore::write_petri_run_row_on_connection(
            &mut tx,
            &parked_run,
            &projection,
        )
        .await
        .expect("run row seed");
        let projection_json = serde_json::to_string(&projection).expect("projection json");
        sqlx::query(
            "INSERT INTO petri_projection (run_id, projection_json, fold_json, \
             positions_json, stream_seq, updated_at_ms) VALUES (?, ?, '{}', '{}', 0, 0) \
             ON CONFLICT(run_id) DO UPDATE SET projection_json = excluded.projection_json",
        )
        .bind(parked_run.to_string())
        .bind(projection_json)
        .execute(&mut *tx)
        .await
        .expect("projection row seed");
        tx.commit().await.expect("seed commit");

        // Lap two: a fresh run asks for the lane. The item's live claim
        // is held by a parked run — the grant attempt must NOT grant.
        let next_run = create_run_with_target(
            &state,
            RunTarget::Git(GitRunTarget {
                repo:   "denkhaus/fabro".to_string(),
                branch: "denkhaus".to_string(),
                tag:    None,
                sha:    None,
            }),
        )
        .await;
        let second = claim_next_for_run(&state, next_run).await;
        assert_eq!(
            second["outcome"], "empty",
            "the parked run's claim holds the item out of the pool: {second}"
        );

        // The parked run's claim is untouched — the burn's re-claim
        // stampede is structurally refused, and the operator's
        // disposition stays the only release.
        let held = state
            .work_store()
            .active_claim_for_run(&parked_run)
            .await
            .expect("claim read")
            .expect("the parked run's claim stays live");
        assert_eq!(held.claim_id, claim_id);
        assert_ne!(
            held.state,
            fabro_work::ClaimState::Released,
            "the park pin: the claim never yields to a fresh lap"
        );
    }

    async fn claim_next_ask(
        state: &Arc<crate::server::AppState>,
        principal: &Principal,
        run_id: RunId,
    ) -> serde_json::Value {
        let response = super::claim_next(
            RequiredRunManagementActor(principal.clone()),
            State(Arc::clone(state)),
            Path("self".to_string()),
            axum::Json(fabro_api::types::WorkClaimNextRequest {
                run_id: run_id.to_string(),
                lane:   "loop".to_string(),
            }),
        )
        .await
        .unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn a_run_that_releases_every_lane_item_gets_an_empty_claim() {
        use chrono::Utc;
        use fabro_work::{TrackerProvider, WorkItem, WorkItemFilter};

        struct TwoItemTracker;
        fn item(key: &str, priority: i64) -> WorkItem {
            WorkItem {
                item_key: key.to_string(),
                title: format!("Item {key}"),
                body: "the spec".to_string(),
                state: "open".to_string(),
                labels: vec!["lane:loop".to_string()],
                priority,
                url: format!("https://github.com/denkhaus/work/issues/{key}"),
                updated_at: Some(Utc::now()),
                created_at: None,
                closed_at: None,
            }
        }
        #[async_trait::async_trait]
        impl TrackerProvider for TwoItemTracker {
            async fn list(
                &self,
                _project: &fabro_work::WorkProject,
                _filter: WorkItemFilter,
            ) -> anyhow::Result<Vec<WorkItem>> {
                Ok(vec![item("denkhaus/work#1", 1), item("denkhaus/work#2", 2)])
            }
            async fn get(
                &self,
                _project: &fabro_work::WorkProject,
                item_key: &str,
            ) -> anyhow::Result<Option<WorkItem>> {
                Ok(Some(item(item_key, 1)))
            }
            async fn create(
                &self,
                _project: &fabro_work::WorkProject,
                _draft: fabro_work::WorkItemDraft,
            ) -> anyhow::Result<WorkItem> {
                anyhow::bail!("not needed")
            }
            async fn update(
                &self,
                _project: &fabro_work::WorkProject,
                _item_key: &str,
                _title: Option<&str>,
                _body: Option<&str>,
            ) -> anyhow::Result<WorkItem> {
                anyhow::bail!("not needed")
            }
            async fn close(
                &self,
                _project: &fabro_work::WorkProject,
                _item_key: &str,
                _reason: &str,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            async fn comment(
                &self,
                _project: &fabro_work::WorkProject,
                _item_key: &str,
                _body: &str,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            async fn project_claim(
                &self,
                _project: &fabro_work::WorkProject,
                _item_key: &str,
                _run_id: &str,
            ) -> anyhow::Result<()> {
                Ok(())
            }
            async fn clear_claim(
                &self,
                _project: &fabro_work::WorkProject,
                _item_key: &str,
                _run_id: &str,
            ) -> anyhow::Result<()> {
                Ok(())
            }
        }

        let state = TestAppStateBuilder::new()
            .env_lookup(|_| None)
            .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
            .work_tracker(Arc::new(TwoItemTracker) as Arc<dyn TrackerProvider>)
            .build();
        state
            .work_store()
            .upsert_project(&bound_project())
            .await
            .unwrap();
        let run_id = create_run_with_target(
            &state,
            RunTarget::Git(GitRunTarget {
                repo:   "denkhaus/fabro".to_string(),
                branch: "denkhaus".to_string(),
                tag:    None,
                sha:    None,
            }),
        )
        .await;
        // claim → release all → claim returns none, no re-grant
        // (denkhaus/work#1186): the same-run release skip is run-lifetime
        // long, so exhausting the lane ends the loop instead of cycling.
        let worker = Principal::Worker { run_id };
        let mut granted_items = Vec::new();
        for _ in 0..2 {
            let body = claim_next_ask(&state, &worker, run_id).await;
            assert_eq!(body["outcome"], "granted", "the lane still has items");
            let claim_id = body["claim"]["claimId"].as_str().unwrap().to_string();
            granted_items.push(body["item"]["itemKey"].as_str().unwrap().to_string());
            state
                .work_store()
                .release(&claim_id, "misroute")
                .await
                .unwrap();
        }
        assert_eq!(
            granted_items,
            vec!["denkhaus/work#1".to_string(), "denkhaus/work#2".to_string()],
            "the run claims each lane item exactly once"
        );
        let body = claim_next_ask(&state, &worker, run_id).await;
        assert_eq!(
            body["outcome"], "empty",
            "a run that released every open lane item gets an empty claim, no re-grant"
        );
    }

    #[tokio::test]
    async fn a_non_run_caller_cannot_use_self() {
        let state = state();
        state
            .work_store()
            .upsert_project(&bound_project())
            .await
            .unwrap();
        let auth = RequiredRunManagementActor(Principal::Webhook {
            delivery_id: "delivery".to_string(),
        });
        let error = resolve_project(&state, &auth, "self")
            .await
            .expect_err("`self` is reserved for run-scoped callers");
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    fn foreign_project() -> fabro_work::WorkProject {
        fabro_work::WorkProject {
            project_id:               "other".to_string(),
            display_name:             "Other".to_string(),
            tracker_provider:         "github".to_string(),
            tracker_repo:             "denkhaus/work".to_string(),
            target_repo:              Some("denkhaus/other".to_string()),
            project_label:            "project:other".to_string(),
            lane_labels:              [("loop".to_string(), "lane:loop".to_string())]
                .into_iter()
                .collect(),
            claim_label_prefix:       "fabro:run:".to_string(),
            claim_comment_enabled:    false,
            backpressure_open_pr_cap: 0,
        }
    }

    /// A run on denkhaus/fabro is scoped to project fabro: `self` and the
    /// literal id both pass, every other project — reads included — is
    /// refused with the scope in the message (denkhaus/work#991).
    #[tokio::test]
    async fn a_worker_is_bound_to_its_own_project_beyond_it_refused() {
        let state = state();
        state
            .work_store()
            .upsert_project(&bound_project())
            .await
            .unwrap();
        state
            .work_store()
            .upsert_project(&foreign_project())
            .await
            .unwrap();
        let run_id = create_run_with_target(
            &state,
            RunTarget::Git(GitRunTarget {
                repo:   "denkhaus/fabro".to_string(),
                branch: "denkhaus".to_string(),
                tag:    None,
                sha:    None,
            }),
        )
        .await;
        let auth = RequiredRunManagementActor(Principal::Worker { run_id });

        let caller = resolve_caller_project(&state, &auth, "self").await.unwrap();
        assert_eq!(caller.project().project_id, "fabro");
        let caller = resolve_caller_project(&state, &auth, "fabro")
            .await
            .unwrap();
        assert_eq!(caller.project().project_id, "fabro");

        let error = resolve_caller_project(&state, &auth, "other")
            .await
            .expect_err("a foreign project is beyond the run's scope — reads included");
        assert_eq!(error.status(), StatusCode::FORBIDDEN);
    }

    /// A user names any project; `self` stays reserved for runs.
    #[tokio::test]
    async fn a_user_resolves_any_project() {
        let state = state();
        state
            .work_store()
            .upsert_project(&bound_project())
            .await
            .unwrap();
        state
            .work_store()
            .upsert_project(&foreign_project())
            .await
            .unwrap();
        let auth = RequiredRunManagementActor(Principal::User(fabro_types::UserPrincipal {
            identity:    fabro_types::IdpIdentity::new("https://issuer.test", "user-1")
                .expect("issuer and subject are non-empty"),
            login:       "tester".to_string(),
            auth_method: fabro_types::AuthMethod::DevToken,
            avatar_url:  None,
        }));
        let caller = resolve_caller_project(&state, &auth, "other")
            .await
            .unwrap();
        assert_eq!(caller.project().project_id, "other");
    }

    /// Claim-owning routes bind to the run: a worker touches only claims
    /// whose run is its own; a user touches any (denkhaus/work#991).
    #[tokio::test]
    async fn a_claim_belongs_to_its_run_for_workers() {
        let state = state();
        state
            .work_store()
            .upsert_project(&bound_project())
            .await
            .unwrap();
        let holder = create_run_with_target(
            &state,
            RunTarget::Git(GitRunTarget {
                repo:   "denkhaus/fabro".to_string(),
                branch: "denkhaus".to_string(),
                tag:    None,
                sha:    None,
            }),
        )
        .await;
        let stranger = RunId::new();
        let granted = state
            .work_store()
            .grant(&fabro_work::GrantRequest {
                project_id: "fabro".to_string(),
                item_key:   "denkhaus/work#7".to_string(),
                run_id:     holder,
                lane:       "loop".to_string(),
            })
            .await
            .unwrap();
        let claim = match granted {
            fabro_work::GrantOutcome::Granted(claim) => claim,
            other => panic!("the fixture grant must succeed: {other:?}"),
        };

        bind_claim_to_caller(
            &RequiredRunManagementActor(Principal::Worker { run_id: holder }),
            &claim,
        )
        .expect("the holding run touches its own claim");
        bind_claim_to_caller(
            &RequiredRunManagementActor(Principal::User(fabro_types::UserPrincipal {
                identity:    fabro_types::IdpIdentity::new("https://issuer.test", "user-1")
                    .expect("issuer and subject are non-empty"),
                login:       "tester".to_string(),
                auth_method: fabro_types::AuthMethod::DevToken,
                avatar_url:  None,
            })),
            &claim,
        )
        .expect("a user touches any claim");

        let error = bind_claim_to_caller(
            &RequiredRunManagementActor(Principal::Worker { run_id: stranger }),
            &claim,
        )
        .expect_err("a foreign run's claim is beyond the worker's scope");
        assert_eq!(error.status(), StatusCode::FORBIDDEN);
    }

    /// grant/next pin the body runId to the authenticated run for
    /// workers: a foreign runId is refused loudly, never stamped over
    /// (denkhaus/work#991).
    #[test]
    fn grant_pins_the_body_run_id_for_workers() {
        let run_a = RunId::new();
        let run_b = RunId::new();
        let caller = WorkCaller::RunWorker {
            run_id:  run_a,
            project: bound_project(),
        };
        assert_eq!(bind_granted_run_id(&caller, run_a).unwrap(), run_a);
        let error = bind_granted_run_id(&caller, run_b)
            .expect_err("a foreign body runId is refused, not stamped over");
        assert_eq!(error.status(), StatusCode::FORBIDDEN);

        let caller = WorkCaller::User {
            project: bound_project(),
        };
        assert_eq!(
            bind_granted_run_id(&caller, run_b).unwrap(),
            run_b,
            "users keep granting for any run"
        );
    }

    /// Close and update are claim-bound for workers: the run must HOLD a
    /// live claim on the item — another run's claim and no claim at all
    /// are both beyond scope (denkhaus/work#991).
    #[tokio::test]
    async fn close_and_update_require_a_live_claim_held_by_the_run() {
        let state = state();
        state
            .work_store()
            .upsert_project(&bound_project())
            .await
            .unwrap();
        let holder = create_run_with_target(
            &state,
            RunTarget::Git(GitRunTarget {
                repo:   "denkhaus/fabro".to_string(),
                branch: "denkhaus".to_string(),
                tag:    None,
                sha:    None,
            }),
        )
        .await;
        let stranger = RunId::new();
        state
            .work_store()
            .grant(&fabro_work::GrantRequest {
                project_id: "fabro".to_string(),
                item_key:   "denkhaus/work#9".to_string(),
                run_id:     holder,
                lane:       "loop".to_string(),
            })
            .await
            .unwrap();

        let holder_caller = WorkCaller::RunWorker {
            run_id:  holder,
            project: bound_project(),
        };
        require_claim_held_by_run(&state, &holder_caller, "denkhaus/work#9")
            .await
            .expect("the holding run closes and edits its claimed item");

        let stranger_caller = WorkCaller::RunWorker {
            run_id:  stranger,
            project: bound_project(),
        };
        let error = require_claim_held_by_run(&state, &stranger_caller, "denkhaus/work#9")
            .await
            .expect_err("an item another run holds is beyond scope");
        assert_eq!(error.status(), StatusCode::FORBIDDEN);

        let error = require_claim_held_by_run(&state, &stranger_caller, "denkhaus/work#404")
            .await
            .expect_err("an unclaimed item is beyond a worker's scope too");
        assert_eq!(error.status(), StatusCode::FORBIDDEN);

        let user_caller = WorkCaller::User {
            project: bound_project(),
        };
        require_claim_held_by_run(&state, &user_caller, "denkhaus/work#9")
            .await
            .expect("a user closes and edits without holding a claim");
    }
}
