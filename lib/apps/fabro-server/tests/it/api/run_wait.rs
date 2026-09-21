//! HTTP-level integration tests for `GET /api/v1/runs/{id}/wait`
//! (fabro-571e, petri port W4-4).
//!
//! Status/validation contracts over the long-poll endpoint: immediate
//! `terminal` results, structured `timeout` carrying the current status,
//! the `until=merged` pull-request precondition, and query validation.
//! The gate-verdict core (dirty/blocked + check runs) is pinned in the
//! handler's unit tests; the merged-mode GitHub wire path rides a later
//! pass.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fabro_server::test_support::build_test_router;
use fabro_types::{RunId, RunStatus, SuccessReason};
use serde_json::json;
use tower::ServiceExt;

use crate::helpers::{api, response_json, response_status};

fn wait_app() -> (
    axum::Router,
    Arc<fabro_server::server::AppState>,
    Arc<fabro_store::Database>,
) {
    let (store, artifact_store) = fabro_server::test_support::test_store_bundle();
    let state = fabro_server::test_support::test_app_state_with_store(
        crate::helpers::test_settings().server_settings,
        crate::helpers::test_settings().manifest_run_defaults,
        5,
        Arc::clone(&store),
        artifact_store,
    );
    let app = build_test_router(state.clone());
    (app, state, store)
}

/// Seed a run row the projector's way: a projection carrying `status`.
/// `completed` seeds `Succeeded { Completed }`, else `Running`.
async fn append_run(store: &Arc<fabro_store::Database>, run_id: RunId, completed: bool) {
    use std::collections::HashMap;
    let status = if completed {
        RunStatus::Succeeded {
            reason: SuccessReason::Completed,
        }
    } else {
        RunStatus::Running
    };
    let mut projection = fabro_types::RunProjection::new(
        "wait wire".to_string(),
        fabro_types::RunSpec {
            run_id,
            settings: fabro_types::WorkflowSettings::default(),
            graph: fabro_types::RunGraph::new("test"),
            graph_source: None,
            workflow_slug: None,
            workflow_version_id: None,
            target: None,
            automation: None,
            source_directory: None,
            labels: HashMap::new(),
            provenance: fabro_types::test_support::test_run_provenance(),
            definition_blob: None,
            spec_blob: None,
            git: None,
            fork_source_ref: None,
            admission: fabro_types::PetriAdmission::default(),
        },
        chrono::Utc::now(),
    );
    projection.status = status;
    let pool = store.run_summary_store().pool();
    let mut tx = pool.begin().await.expect("seed tx");
    fabro_store::RunSummaryStore::write_petri_run_row_on_connection(&mut tx, &run_id, &projection)
        .await
        .expect("seed run row");
    let projection_json = serde_json::to_string(&projection).expect("projection json");
    sqlx::query(
        "INSERT INTO petri_projection (run_id, projection_json, fold_json, positions_json, \
         stream_seq, updated_at_ms) VALUES (?, ?, '{}', '{}', 0, 0) \
         ON CONFLICT(run_id) DO UPDATE SET projection_json = excluded.projection_json",
    )
    .bind(run_id.to_string())
    .bind(projection_json)
    .execute(&mut *tx)
    .await
    .expect("seed projection row");
    tx.commit().await.expect("seed commit");
}

#[tokio::test]
async fn terminal_run_returns_terminal_immediately() {
    let (app, _state, store) = wait_app();
    let run_id = RunId::new();
    append_run(&store, run_id, true).await;
    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/wait?until=terminal&timeout_ms=2000"
        )))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = response_json(resp, StatusCode::OK, "GET /api/v1/runs/{id}/wait terminal").await;
    assert_eq!(body["reached"].as_str(), Some("terminal"));
    assert_eq!(body["status"]["kind"].as_str(), Some("succeeded"));
    assert_eq!(body["run_id"].as_str(), Some(run_id.to_string().as_str()));
}

#[tokio::test]
async fn running_run_returns_structured_timeout_with_current_status() {
    let (app, _state, store) = wait_app();
    let run_id = RunId::new();
    append_run(&store, run_id, false).await;
    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/wait?until=terminal&timeout_ms=100"
        )))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = response_json(resp, StatusCode::OK, "GET /api/v1/runs/{id}/wait timeout").await;
    assert_eq!(body["reached"].as_str(), Some("timeout"));
    assert_eq!(body["status"]["kind"].as_str(), Some("running"));
}

#[tokio::test]
async fn merged_wait_without_pull_request_returns_404() {
    let (app, _state, store) = wait_app();
    let run_id = RunId::new();
    append_run(&store, run_id, false).await;
    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/wait?until=merged&timeout_ms=100"
        )))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    response_status(
        resp,
        StatusCode::NOT_FOUND,
        "GET /api/v1/runs/{id}/wait merged without PR",
    )
    .await;
}

#[tokio::test]
async fn out_of_range_timeout_ms_returns_400() {
    let (app, _state, _store) = wait_app();
    let run_id = RunId::new();
    for timeout_ms in [0u64, 3_600_001] {
        let req = Request::builder()
            .method("GET")
            .uri(api(&format!(
                "/runs/{run_id}/wait?until=terminal&timeout_ms={timeout_ms}"
            )))
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        response_status(
            resp,
            StatusCode::BAD_REQUEST,
            format!("GET /api/v1/runs/{run_id}/wait timeout_ms={timeout_ms}"),
        )
        .await;
    }
}

#[tokio::test]
async fn unknown_until_value_returns_400() {
    let (app, _state, _store) = wait_app();
    let run_id = RunId::new();
    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/wait?until=nonsense&timeout_ms=100"
        )))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    response_status(
        resp,
        StatusCode::BAD_REQUEST,
        "GET /api/v1/runs/{id}/wait invalid until",
    )
    .await;
}
