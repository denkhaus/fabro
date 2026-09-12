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
    let inventory = state.sandbox_inventory().list_managed().await;
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
        .filter_map(|info| info.status.labels.get(RUN_ID_LABEL).cloned())
        .collect::<Vec<_>>();
    run_ids.sort();
    run_ids.dedup();
    Ok(Json(RunSandboxAvailability { run_ids }))
}

async fn list_sandboxes(
    State(state): State<Arc<AppState>>,
    _auth: RequiredRunManagementActor,
) -> Json<SandboxListResponse> {
    Json(state.sandbox_inventory().list_managed().await)
}

async fn retrieve_sandbox(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    _auth: RequiredRunManagementActor,
) -> Result<Json<SandboxInfo>, ApiError> {
    state
        .sandbox_inventory()
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
    use fabro_sandbox::SandboxInventory;
    use fabro_sandbox::driver::{ConnectedProvider, ProviderConnectOptions};
    use fabro_sandbox::test_support::{
        managed_scripted_sandbox, managed_scripted_sandbox_for_run, scripted_inventory_provider,
    };
    use fabro_types::SandboxProviderKind;
    use fabro_types::settings::server::{SandboxPluginSettings, ServerSandboxProviderSettings};
    use serde_json::{Value, json};
    use tower::ServiceExt;

    use crate::test_support::{TestAppStateBuilder, build_test_router};

    fn app_with_inventory(inventory: SandboxInventory) -> axum::Router {
        let state = TestAppStateBuilder::new()
            .sandbox_inventory(inventory)
            .build();
        build_test_router(state)
    }

    /// A connected provider of `kind` holding fabro-managed sandboxes `ids`.
    fn provider(kind: SandboxProviderKind, ids: &[&str]) -> ConnectedProvider {
        scripted_inventory_provider(
            kind,
            ids.iter().map(|id| managed_scripted_sandbox(id)).collect(),
        )
    }

    /// A plugin kind whose executable does not exist, so every lookup fails
    /// to connect.
    fn with_unreachable_plugin(inventory: SandboxInventory, name: &str) -> SandboxInventory {
        let settings = ServerSandboxProviderSettings {
            enabled: true,
            plugin:  Some(SandboxPluginSettings {
                path: Some(format!("/nonexistent/fabro-sandbox-{name}")),
                dev: true,
                ..SandboxPluginSettings::default()
            }),
        };
        inventory.with_lazy(
            SandboxProviderKind::try_new(name).expect("valid kind"),
            settings,
            ProviderConnectOptions::default(),
        )
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
        let run_one = managed_scripted_sandbox_for_run("container-1", "01M1EGK6J8CNP9APJ7PWE6WE74");
        let run_two = managed_scripted_sandbox_for_run("snapshot-9", "01M0WWKAQCWZC0Q0JK019H0ZC7");
        // Same run reachable through a second provider entry: deduplicated.
        let duplicate =
            managed_scripted_sandbox_for_run("container-2", "01M0WWKAQCWZC0Q0JK019H0ZC7");
        // Non-run sandboxes (no run label) never appear.
        let unscoped = managed_scripted_sandbox("container-3");

        let docker = scripted_inventory_provider(SandboxProviderKind::DOCKER, vec![
            run_one, duplicate, unscoped,
        ]);
        let daytona = scripted_inventory_provider(SandboxProviderKind::DAYTONA, vec![run_two]);
        let inventory = SandboxInventory::empty()
            .with_connected(docker)
            .with_connected(daytona);

        let app = app_with_inventory(inventory);
        let response = app
            .oneshot(req_get("/api/v1/runs/sandbox-availability"))
            .await
            .expect("availability request dispatches");
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(
            body,
            json!({
                "run_ids": [
                    "01M0WWKAQCWZC0Q0JK019H0ZC7",
                    "01M1EGK6J8CNP9APJ7PWE6WE74",
                ]
            })
        );
    }

    #[tokio::test]
    async fn sandbox_availability_fails_closed_on_provider_error() {
        let inventory = with_unreachable_plugin(SandboxInventory::default(), "broken");

        let app = app_with_inventory(inventory);
        let response = app
            .oneshot(req_get("/api/v1/runs/sandbox-availability"))
            .await
            .expect("availability request dispatches");
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn list_returns_provider_backed_data_without_run_projection_state() {
        let app = app_with_inventory(
            SandboxInventory::empty()
                .with_connected(provider(SandboxProviderKind::DOCKER, &["docker-native-id"])),
        );

        let response = app.oneshot(req_get("/api/v1/sandboxes")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["data"][0]["status"]["id"], "docker-native-id");
        assert_eq!(body["data"][0]["provider"], "docker");
        assert_eq!(body["data"][0]["status"]["state"], "running");
        assert_eq!(body["meta"]["provider_errors"], json!([]));
    }

    #[tokio::test]
    async fn retrieve_searches_all_configured_providers() {
        let app = app_with_inventory(
            SandboxInventory::empty()
                .with_connected(provider(SandboxProviderKind::DOCKER, &[]))
                .with_connected(provider(SandboxProviderKind::DAYTONA, &["native-id"])),
        );

        let response = app
            .oneshot(req_get("/api/v1/sandboxes/native-id"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = body_json(response).await;
        assert_eq!(body["status"]["id"], "native-id");
        assert_eq!(body["provider"], "daytona");
    }

    #[tokio::test]
    async fn no_matching_sandbox_returns_404() {
        let app = app_with_inventory(
            SandboxInventory::empty()
                .with_connected(provider(SandboxProviderKind::DOCKER, &[]))
                .with_connected(provider(SandboxProviderKind::DAYTONA, &[])),
        );

        let response = app
            .oneshot(req_get("/api/v1/sandboxes/missing"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn duplicate_native_ids_return_409() {
        let app = app_with_inventory(
            SandboxInventory::empty()
                .with_connected(provider(SandboxProviderKind::DOCKER, &["same-id"]))
                .with_connected(provider(SandboxProviderKind::DAYTONA, &["same-id"])),
        );

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
        let app = app_with_inventory(with_unreachable_plugin(
            SandboxInventory::empty().with_connected(provider(SandboxProviderKind::DOCKER, &[])),
            "e2b",
        ));

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
                .contains("e2b: Failed to connect to the e2b provider")
        );
    }
}
