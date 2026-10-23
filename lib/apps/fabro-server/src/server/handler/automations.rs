use std::sync::Arc;

use axum::http::HeaderMap;
use axum_extra::extract::Query as ExtraQuery;
use fabro_automation::{
    Automation, AutomationDraft, AutomationId, AutomationOverlapPolicy, AutomationReplace,
    AutomationStoreError,
};
use fabro_environment::EnvironmentId;
use fabro_store::{RunSummaryListQuery, RunSummaryVisibility};
use fabro_types::{AutomationRef, RunId};
use fabro_util::error as error_util;
use serde::Serialize;

use super::super::{
    ApiError, AppState, IntoResponse, Json, PaginationParams, Path, RequiredUser, Response, Router,
    State, StatusCode, clamp_page_limit, clamp_page_offset, get,
};
use super::{json_with_etag_response, lifecycle, parse_required_if_match, runs};
use crate::automation_materializer::AutomationRunMaterializeInput;
use crate::principal_middleware::RequiredRunToolActor;
use crate::run_manifest;

#[derive(Serialize)]
struct AutomationListResponse {
    data: Vec<Automation>,
    meta: AutomationListMeta,
}

#[derive(Serialize)]
struct AutomationListMeta {
    total: usize,
}

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/automations",
            get(list_automations).post(create_automation),
        )
        .route(
            "/automations/{id}/runs",
            get(list_automation_runs).post(create_automation_run),
        )
        .route(
            "/automations/{id}",
            get(get_automation)
                .put(replace_automation)
                .delete(delete_automation),
        )
}

async fn list_automations(
    _auth: RequiredUser,
    State(state): State<Arc<AppState>>,
) -> Result<Response, ApiError> {
    let data = state.automation_store().list().await?;
    let total = data.len();
    Ok((
        StatusCode::OK,
        Json(AutomationListResponse {
            data,
            meta: AutomationListMeta { total },
        }),
    )
        .into_response())
}

async fn list_automation_runs(
    _auth: RequiredUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    ExtraQuery(pagination): ExtraQuery<PaginationParams>,
) -> Response {
    let id = match parse_path_id(id) {
        Ok(id) => id,
        Err(err) => return err.into_response(),
    };
    match state.automation_store().exists(&id).await {
        Ok(true) => {}
        Ok(false) => {
            return ApiError::not_found(format!("automation not found: {id}")).into_response();
        }
        Err(err) => return ApiError::from(err).into_response(),
    }

    let query = RunSummaryListQuery {
        automation_id: Some(id.to_string()),
        visibility: RunSummaryVisibility::All,
        limit: clamp_page_limit(pagination.limit),
        offset: clamp_page_offset(pagination.offset),
        ..RunSummaryListQuery::default()
    };
    runs::run_summary_page_response(&state, &query).await
}

async fn create_automation_run(
    RequiredRunToolActor(actor): RequiredRunToolActor,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let id = match parse_path_id(id) {
        Ok(id) => id,
        Err(err) => return err.into_response(),
    };
    let automation = match state.automation_store().get(&id).await {
        Ok(Some(automation)) => automation,
        Ok(None) => {
            return ApiError::not_found(format!("automation not found: {id}")).into_response();
        }
        Err(err) => return ApiError::from(err).into_response(),
    };
    let Some(api_trigger) = automation.enabled_api_trigger() else {
        return ApiError::with_code(
            StatusCode::CONFLICT,
            "automation has no enabled API trigger",
            "automation_api_trigger_disabled",
        )
        .into_response();
    };
    let api_trigger_id = api_trigger.id.to_string();
    let environment_id = match resolve_automation_environment(
        state.as_ref(),
        automation.environment_id.as_deref(),
        StatusCode::CONFLICT,
    ) {
        Ok(environment_id) => environment_id,
        Err(err) => return err.into_response(),
    };
    let Some(target) = automation.git_target().cloned() else {
        return ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Stored automation target is not Git-backed",
        )
        .into_response();
    };

    let run_id = RunId::new();
    let materialized = match state
        .materialize_automation_run(AutomationRunMaterializeInput {
            automation_id: automation.id.clone(),
            target,
            workflow_source: automation.workflow_source.clone(),
            workflow: automation.workflow.clone(),
            run_id,
            temp_root: state.automation_temp_root(),
        })
        .await
    {
        Ok(materialized) => materialized,
        Err(err) => {
            let message = error_util::collect_chain(&err).join(": ");
            return ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, message).into_response();
        }
    };
    let automation_ref = AutomationRef {
        id:              automation.id.to_string(),
        name:            Some(automation.name.clone()),
        trigger_id:      Some(api_trigger_id),
        workflow_source: materialized.workflow_source.clone(),
    };

    let response = Box::pin(runs::create_run_from_intent(
        Arc::clone(&state),
        runs::CreateRunFromIntentRequest {
            intent: materialized.into_run_intent(environment_id),
            explicit_run_id: Some(run_id),
            actor: actor.clone(),
            headers,
            automation: Some(automation_ref),
        },
    ))
    .await;

    // An automation's API trigger should both create and start the run; otherwise
    // the run sits in `Submitted` forever because the scheduler only claims
    // `Runnable`. Mirror what the UI does for a manual create-then-start flow.
    if response.status().is_success() {
        if let Err(err) = lifecycle::queue_run_start(state.as_ref(), run_id, false, actor).await {
            tracing::warn!(
                %run_id,
                automation_id = %automation.id,
                error = ?err,
                "Created automation run but failed to start it",
            );
        }
    }

    response
}

async fn create_automation(
    _auth: RequiredUser,
    State(state): State<Arc<AppState>>,
    Json(mut draft): Json<AutomationDraft>,
) -> Result<Response, ApiError> {
    draft.environment_id = Some(resolve_automation_environment(
        state.as_ref(),
        draft.environment_id.as_deref(),
        StatusCode::UNPROCESSABLE_ENTITY,
    )?);
    let Some(work_project) = draft
        .work_project
        .as_deref()
        .map(str::trim)
        .filter(|project| !project.is_empty())
    else {
        return Err(ApiError::with_code(
            StatusCode::UNPROCESSABLE_ENTITY,
            "automation work_project is required",
            "automation_work_project_required",
        ));
    };
    require_bound_work_project(state.as_ref(), work_project).await?;
    // A definition written through the API always carries a visible overlap
    // policy: an omitted `on_overlap` becomes an explicit `Skip` instead of
    // an untagged `None`, so replaces never silently drop the field
    // (fabro-2093). Fire stays explicit user intent.
    draft.on_overlap = Some(draft.on_overlap.unwrap_or(AutomationOverlapPolicy::Skip));
    let automation = state.automation_store().create(draft).await?;
    state.notify_automation_scheduler();
    Ok((StatusCode::CREATED, Json(automation)).into_response())
}

async fn get_automation(
    _auth: RequiredUser,
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let id = parse_path_id(id)?;
    match state.automation_store().get(&id).await? {
        Some(automation) => Ok(automation_with_etag_response(StatusCode::OK, automation)),
        None => Err(ApiError::not_found(format!("automation not found: {id}"))),
    }
}

async fn replace_automation(
    _auth: RequiredUser,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(mut replacement): Json<AutomationReplace>,
) -> Result<Response, ApiError> {
    let id = parse_path_id(id)?;
    let expected = parse_required_if_match(&headers, "automation", &id)?;
    replacement.environment_id = Some(resolve_automation_environment(
        state.as_ref(),
        replacement.environment_id.as_deref(),
        StatusCode::UNPROCESSABLE_ENTITY,
    )?);
    // A replace that omits the binding keeps the stored one, so partial
    // edits (pausing a schedule) never silently unbind a firing automation.
    if replacement.work_project.is_none() {
        replacement.work_project = state
            .automation_store()
            .get(&id)
            .await?
            .and_then(|current| current.work_project);
    }
    if let Some(work_project) = replacement.work_project.as_deref() {
        require_bound_work_project(state.as_ref(), work_project).await?;
    }
    // See create_automation: replaces persist a visible overlap policy.
    replacement.on_overlap = Some(
        replacement
            .on_overlap
            .unwrap_or(AutomationOverlapPolicy::Skip),
    );
    let automation = state
        .automation_store()
        .replace(&id, &expected, replacement)
        .await?;
    state.notify_automation_scheduler();
    Ok(automation_with_etag_response(StatusCode::OK, automation))
}

async fn delete_automation(
    _auth: RequiredUser,
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let id = parse_path_id(id)?;
    let expected = parse_required_if_match(&headers, "automation", &id)?;
    state.automation_store().delete(&id, &expected).await?;
    state.notify_automation_scheduler();
    Ok(StatusCode::NO_CONTENT.into_response())
}

fn parse_path_id(id: String) -> Result<AutomationId, ApiError> {
    AutomationId::new(id)
        .map_err(|err| ApiError::bad_request(format!("invalid automation id: {err}")))
}

pub(in crate::server) fn resolve_automation_environment(
    state: &AppState,
    value: Option<&str>,
    status: StatusCode,
) -> Result<String, ApiError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Err(ApiError::with_code(
            status,
            "automation environment is required",
            "automation_environment_required",
        ));
    };
    let id = value.parse::<EnvironmentId>().map_err(|_| {
        ApiError::with_code(
            status,
            "automation environment id is invalid",
            "automation_environment_invalid",
        )
    })?;
    let Some(environment) = state.environment_store().get(&id) else {
        return Err(ApiError::with_code(
            status,
            format!("automation environment not found: {id}"),
            "automation_environment_not_found",
        ));
    };
    if !environment.settings.provider.clones_workspace() {
        return Err(ApiError::with_code(
            status,
            format!(
                "automation environment `{id}` is incompatible; Git-backed automations require Docker or Daytona"
            ),
            "automation_environment_incompatible",
        ));
    }
    let provider = environment.settings.provider.clone();
    if let Some(message) =
        run_manifest::sandbox_provider_policy_error(&state.server_settings(), &provider)
    {
        return Err(ApiError::with_code(
            status,
            message,
            "automation_environment_provider_disabled",
        ));
    }
    if !state
        .sandbox_inventory()
        .kinds()
        .any(|kind| *kind == provider)
    {
        return Err(ApiError::with_code(
            status,
            format!("sandbox provider `{provider}` is not ready on this server"),
            "automation_environment_provider_unavailable",
        ));
    }
    Ok(id.to_string())
}

/// fabro-f1a9: an automation names the work project its runs claim through.
/// The save path validates the binding so a typo cannot ship an automation
/// the scheduler would then refuse to fire (the overnight outage class: a
/// dead tracker binding burning scheduled runs).
async fn require_bound_work_project(state: &AppState, project_id: &str) -> Result<(), ApiError> {
    let project = state
        .work_store()
        .project(project_id)
        .await
        .map_err(|error| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("work store operation failed: {error}"),
            )
        })?;
    match project {
        Some(project)
            if !project.tracker_provider.trim().is_empty()
                && !project.tracker_repo.trim().is_empty() =>
        {
            Ok(())
        }
        Some(_) => Err(ApiError::with_code(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("work project `{project_id}` has no tracker wiring"),
            "automation_work_project_unconfigured",
        )),
        None => Err(ApiError::with_code(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("work project not found: {project_id}"),
            "automation_work_project_not_found",
        )),
    }
}

fn automation_with_etag_response(status: StatusCode, automation: Automation) -> Response {
    let revision = automation.revision.clone();
    json_with_etag_response(status, "automation", &revision, automation)
}

impl From<AutomationStoreError> for ApiError {
    fn from(err: AutomationStoreError) -> Self {
        match err {
            AutomationStoreError::NotFound { id } => {
                Self::not_found(format!("automation not found: {id}"))
            }
            AutomationStoreError::EnvironmentNotFound { environment } => Self::with_code(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("automation environment not found: {environment}"),
                "automation_environment_not_found",
            ),
            AutomationStoreError::AlreadyExists { id } => Self::new(
                StatusCode::CONFLICT,
                format!("automation already exists: {id}"),
            ),
            AutomationStoreError::StaleRevision { id, .. } => Self::new(
                StatusCode::CONFLICT,
                format!("automation revision is stale: {id}"),
            ),
            AutomationStoreError::Validation { source } => {
                Self::new(StatusCode::UNPROCESSABLE_ENTITY, source.to_string())
            }
            err => {
                tracing::error!(error = ?err, "Automation store operation failed");
                Self::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "automation store operation failed",
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use crate::server::AppState;
    use crate::test_support::{TestAppStateBuilder, build_test_router};

    /// A state whose registry holds the named, fully wired projects.
    async fn app_with_projects(projects: &[&str]) -> (Arc<AppState>, axum::Router) {
        let state = TestAppStateBuilder::new().build();
        for project_id in projects {
            state
                .stores
                .work
                .upsert_project(&fabro_work::WorkProject {
                    project_id:               (*project_id).to_owned(),
                    display_name:             (*project_id).to_owned(),
                    tracker_provider:         "github".to_owned(),
                    tracker_repo:             "denkhaus/work".to_owned(),
                    target_repo:              None,
                    project_label:            format!("project:{project_id}"),
                    lane_labels:              BTreeMap::new(),
                    claim_label_prefix:       "fabro:run:".to_owned(),
                    claim_comment_enabled:    false,
                    backpressure_open_pr_cap: 0,
                })
                .await
                .expect("test project upserts");
        }
        (Arc::clone(&state), build_test_router(state))
    }

    fn create_body(id: &str, work_project: Option<&str>) -> Value {
        let mut body = json!({
            "id": id,
            "name": id,
            "environment_id": "default",
            "target": { "kind": "git", "repo": "fabro-sh/fabro", "branch": "main" },
            "workflow": "workflow.fabro",
            "triggers": [{ "id": "manual", "type": "api", "enabled": true }],
        });
        if let Some(project) = work_project {
            body["work_project"] = json!(project);
        }
        body
    }

    fn replace_body(work_project: Option<&str>) -> Value {
        let mut body = json!({
            "name": "renamed",
            "environment_id": "default",
            "target": { "kind": "git", "repo": "fabro-sh/fabro", "branch": "main" },
            "workflow": "workflow.fabro",
            "triggers": [{ "id": "manual", "type": "api", "enabled": true }],
        });
        if let Some(project) = work_project {
            body["work_project"] = json!(project);
        }
        body
    }

    async fn send(app: &axum::Router, request: Request<Body>) -> (StatusCode, Value) {
        let response = app.clone().oneshot(request).await.expect("router answers");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body reads");
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).expect("json body")
        };
        (status, value)
    }

    /// The machine-readable code of the first error entry, if any.
    fn error_code(body: &Value) -> Option<&str> {
        body["errors"][0]["code"].as_str()
    }

    fn post(body: &Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/v1/automations")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("request builds")
    }

    fn put(id: &str, revision: Option<&str>, body: &Value) -> Request<Body> {
        let mut builder = Request::builder()
            .method("PUT")
            .uri(format!("/api/v1/automations/{id}"))
            .header("content-type", "application/json");
        if let Some(revision) = revision {
            builder = builder.header("if-match", format!("\"{revision}\""));
        }
        builder
            .body(Body::from(body.to_string()))
            .expect("request builds")
    }

    /// fabro-f1a9: creating an automation names the work project its runs
    /// claim through, and the named project must be registered and wired.
    #[tokio::test]
    async fn create_requires_a_registered_wired_work_project() {
        let (_state, app) = app_with_projects(&["fabro"]).await;

        let (status, body) = send(&app, post(&create_body("unbound", None))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error_code(&body), Some("automation_work_project_required"));

        let (status, body) = send(&app, post(&create_body("absent", Some("absent")))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error_code(&body), Some("automation_work_project_not_found"));

        let (status, body) = send(&app, post(&create_body("bound", Some("fabro")))).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["work_project"], "fabro");
    }

    /// A replace that omits the binding preserves the stored one (a pause
    /// must never unbind a firing automation), and a replace naming an
    /// unregistered project is refused.
    #[tokio::test]
    async fn replace_keeps_the_stored_binding_and_refuses_an_unknown_one() {
        let (_state, app) = app_with_projects(&["fabro"]).await;
        let (status, created) = send(&app, post(&create_body("nightly", Some("fabro")))).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let revision = created["revision"].as_str().expect("revision").to_owned();

        let (status, replaced) =
            send(&app, put("nightly", Some(&revision), &replace_body(None))).await;
        assert_eq!(status, StatusCode::OK, "{replaced}");
        assert_eq!(replaced["work_project"], "fabro");
        let revision = replaced["revision"].as_str().expect("revision").to_owned();

        let (status, body) = send(
            &app,
            put("nightly", Some(&revision), &replace_body(Some("absent"))),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error_code(&body), Some("automation_work_project_not_found"));
    }
}
