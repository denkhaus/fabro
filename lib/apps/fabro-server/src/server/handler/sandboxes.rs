use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use fabro_sandbox::{RUN_ID_LABEL, SandboxLookupError};
use fabro_types::{RunSandboxAvailability, SandboxInfo, SandboxListResponse, SandboxProviderKind};
use tracing::warn;

use super::super::AppState;
use crate::error::ApiError;
use crate::principal_middleware::RequiredRunManagementActor;

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/sandboxes", get(list_sandboxes))
        .route("/sandboxes/{id}", get(retrieve_sandbox))
        .route(
            "/runs/sandbox-availability",
            get(list_run_sandbox_availability),
        )
}

/// Narrow live probe for run-management callers (fabro-8d30a): which run
/// sandboxes still exist on this host. Stopped counts, removed does not.
/// A provider that fails to answer fails the whole request — an
/// incomplete set would read absent runs as "removed".
async fn list_run_sandbox_availability(
    State(state): State<Arc<AppState>>,
    _auth: RequiredRunManagementActor,
) -> Result<Json<RunSandboxAvailability>, ApiError> {
    let inventory = state.sandbox_provider_registry().list_managed().await;
    if !inventory.meta.provider_errors.is_empty() {
        let failures = provider_errors_summary(&inventory.meta.provider_errors);
        warn!(failures = %failures, "Sandbox availability probe incomplete");
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!("Sandbox availability is incomplete: {failures}."),
        ));
    }
    let mut run_ids = inventory
        .data
        .iter()
        .filter_map(|info| info.labels.get(RUN_ID_LABEL).cloned())
        .collect::<Vec<_>>();
    run_ids.sort();
    run_ids.dedup();
    Ok(Json(RunSandboxAvailability { run_ids }))
}

async fn list_sandboxes(
    State(state): State<Arc<AppState>>,
    _auth: RequiredRunManagementActor,
) -> Json<SandboxListResponse> {
    Json(state.sandbox_provider_registry().list_managed().await)
}

async fn retrieve_sandbox(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    _auth: RequiredRunManagementActor,
) -> Result<Json<SandboxInfo>, ApiError> {
    state
        .sandbox_provider_registry()
        .get_managed_by_native_id(&id)
        .await
        .map(Json)
        .map_err(sandbox_lookup_error)
}

fn provider_errors_summary(errors: &[fabro_types::SandboxProviderLookupError]) -> String {
    errors
        .iter()
        .map(|error| format!("{}: {}", error.provider, error.message))
        .collect::<Vec<_>>()
        .join("; ")
}

fn sandbox_lookup_error(err: SandboxLookupError) -> ApiError {
    match err {
        SandboxLookupError::NotFound { id } => ApiError::new(
            StatusCode::NOT_FOUND,
            format!("No provider found a Fabro-managed sandbox with id '{id}'."),
        ),
        SandboxLookupError::Conflict { id, providers } => ApiError::new(
            StatusCode::CONFLICT,
            format!(
                "More than one provider matched sandbox id '{id}': {}.",
                provider_list(&providers)
            ),
        ),
        SandboxLookupError::ProviderUnavailable {
            id,
            provider_errors,
        } => ApiError::new(
            StatusCode::BAD_GATEWAY,
            format!(
                "Provider lookup for sandbox id '{id}' failed before a definitive result could be determined: {}.",
                provider_errors
                    .iter()
                    .map(|error| format!("{}: {}", error.provider, error.message))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        ),
    }
}

fn provider_list(providers: &[SandboxProviderKind]) -> String {
    providers
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use fabro_sandbox::test_support::{
        FakeGet, FakeList, FakeSandboxProvider, fake_registry, fake_sandbox_info,
    };
    use fabro_sandbox::{RUN_ID_LABEL, SandboxProviderRegistry};
    use fabro_types::SandboxProviderKind;
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use crate::test_support::{TestAppStateBuilder, build_test_router};

    fn app_with_registry(registry: SandboxProviderRegistry) -> axum::Router {
        let state = TestAppStateBuilder::new()
            .sandbox_provider_registry(registry)
            .build();
        build_test_router(state)
    }

    fn req_get(uri: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .body(Body::empty())
            .expect("sandbox inventory GET request should build")
    }

    async fn body_json(response: axum::response::Response) -> Value {
        let bytes = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body should fit in memory");
        serde_json::from_slice(&bytes).expect("response body should be valid JSON")
    }

    #[tokio::test]
    async fn sandbox_availability_extracts_sorted_deduped_run_ids() {
        let mut docker_alive = fake_sandbox_info(SandboxProviderKind::Docker, "container-1");
        docker_alive.labels.insert(
            RUN_ID_LABEL.to_string(),
            "01M1EGK6J8CNP9APJ7PWE6WE74".to_string(),
        );
        let mut daytona_alive = fake_sandbox_info(SandboxProviderKind::Daytona, "snapshot-9");
        daytona_alive.labels.insert(
            RUN_ID_LABEL.to_string(),
            "01M0WWKAQCWZC0Q0JK019H0ZC7".to_string(),
        );
        // Same run reachable through a second provider entry: deduplicated.
        let mut duplicate = fake_sandbox_info(SandboxProviderKind::Docker, "container-2");
        duplicate.labels.insert(
            RUN_ID_LABEL.to_string(),
            "01M0WWKAQCWZC0Q0JK019H0ZC7".to_string(),
        );
        // Non-run sandboxes (no run label) never appear.
        let unmanaged_scope = fake_sandbox_info(SandboxProviderKind::Docker, "container-3");
        let app = app_with_registry(fake_registry(vec![FakeSandboxProvider::new(
            SandboxProviderKind::Docker,
            FakeList::Ok(vec![unmanaged_scope, duplicate, docker_alive]),
            FakeGet::Missing,
        )]));

        let response = app
            .oneshot(req_get("/api/v1/runs/sandbox-availability"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(
            body["run_ids"],
            json!(["01M0WWKAQCWZC0Q0JK019H0ZC7", "01M1EGK6J8CNP9APJ7PWE6WE74"])
        );
    }

    #[tokio::test]
    async fn sandbox_availability_fails_closed_on_provider_error() {
        let app = app_with_registry(fake_registry(vec![FakeSandboxProvider::new(
            SandboxProviderKind::Docker,
            FakeList::Err("daemon unreachable"),
            FakeGet::Missing,
        )]));

        let response = app
            .oneshot(req_get("/api/v1/runs/sandbox-availability"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = body_json(response).await;
        let message = body["errors"][0]["detail"].as_str().unwrap_or_default();
        assert!(
            message.contains("incomplete"),
            "502 must name the incomplete view: {message}"
        );
    }

    #[tokio::test]
    async fn list_returns_provider_backed_data_without_run_projection_state() {
        let docker = fake_sandbox_info(SandboxProviderKind::Docker, "docker-native-id");
        let app = app_with_registry(fake_registry(vec![FakeSandboxProvider::new(
            SandboxProviderKind::Docker,
            FakeList::Ok(vec![docker]),
            FakeGet::Missing,
        )]));

        let response = app.oneshot(req_get("/api/v1/sandboxes")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["data"][0]["id"], "docker-native-id");
        assert_eq!(body["data"][0]["provider"], "docker");
        assert_eq!(body["meta"]["provider_errors"], json!([]));
    }

    #[tokio::test]
    async fn retrieve_searches_all_configured_providers() {
        let daytona = fake_sandbox_info(SandboxProviderKind::Daytona, "native-id");
        let app = app_with_registry(fake_registry(vec![
            FakeSandboxProvider::new(
                SandboxProviderKind::Docker,
                FakeList::Ok(Vec::new()),
                FakeGet::Missing,
            ),
            FakeSandboxProvider::new(
                SandboxProviderKind::Daytona,
                FakeList::Ok(Vec::new()),
                FakeGet::Found(Box::new(daytona)),
            ),
        ]));

        let response = app
            .oneshot(req_get("/api/v1/sandboxes/native-id"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["id"], "native-id");
        assert_eq!(body["provider"], "daytona");
    }

    #[tokio::test]
    async fn no_matching_sandbox_returns_404() {
        let app = app_with_registry(fake_registry(vec![
            FakeSandboxProvider::new(
                SandboxProviderKind::Docker,
                FakeList::Ok(Vec::new()),
                FakeGet::Missing,
            ),
            FakeSandboxProvider::new(
                SandboxProviderKind::Daytona,
                FakeList::Ok(Vec::new()),
                FakeGet::Missing,
            ),
        ]));

        let response = app
            .oneshot(req_get("/api/v1/sandboxes/missing"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn duplicate_native_ids_return_409() {
        let app = app_with_registry(fake_registry(vec![
            FakeSandboxProvider::new(
                SandboxProviderKind::Docker,
                FakeList::Ok(Vec::new()),
                FakeGet::Found(Box::new(fake_sandbox_info(
                    SandboxProviderKind::Docker,
                    "same-id",
                ))),
            ),
            FakeSandboxProvider::new(
                SandboxProviderKind::Daytona,
                FakeList::Ok(Vec::new()),
                FakeGet::Found(Box::new(fake_sandbox_info(
                    SandboxProviderKind::Daytona,
                    "same-id",
                ))),
            ),
        ]));

        let response = app
            .oneshot(req_get("/api/v1/sandboxes/same-id"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert!(
            body["errors"][0]["detail"]
                .as_str()
                .unwrap_or_default()
                .contains("More than one provider matched")
        );
    }

    #[tokio::test]
    async fn provider_lookup_uncertainty_returns_502() {
        let app = app_with_registry(fake_registry(vec![
            FakeSandboxProvider::new(
                SandboxProviderKind::Docker,
                FakeList::Ok(Vec::new()),
                FakeGet::Missing,
            ),
            FakeSandboxProvider::new(
                SandboxProviderKind::Daytona,
                FakeList::Ok(Vec::new()),
                FakeGet::Err("daytona unavailable"),
            ),
        ]));

        let response = app
            .oneshot(req_get("/api/v1/sandboxes/maybe-missing"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        let body = body_json(response).await;
        assert!(
            body["errors"][0]["detail"]
                .as_str()
                .unwrap_or_default()
                .contains("daytona unavailable")
        );
    }
}
