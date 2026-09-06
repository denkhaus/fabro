//! HTTP-level integration tests for `GET /api/v1/runs/{id}/wait`
//! (fabro-571e).
//!
//! These tests exercise the long-poll handler's observable contracts:
//! immediate `terminal` results for finished runs, structured `timeout`
//! results carrying the current status, the `until=merged` pull-request
//! precondition, and query validation. GitHub-backed merge detection is
//! covered by the fabro-github fetch tests plus the handler's wiring;
//! these tests do not reach the network.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fabro_server::test_support::test_app_state_with_store;
use fabro_store::{ArtifactStore, Database};
use fabro_types::{Graph, RunId, WorkflowSettings, test_support};
use fabro_workflow::event as workflow_event;
use fabro_workflow::run_status::SuccessReason;
use object_store::memory::InMemory as MemoryObjectStore;
use tower::ServiceExt;

use crate::helpers::{api, response_json, response_status, test_settings};

fn store_bundle() -> (Arc<Database>, ArtifactStore) {
    let object_store: Arc<dyn object_store::ObjectStore> = Arc::new(MemoryObjectStore::new());
    let store = Arc::new(fabro_store::test_support::test_database(
        Arc::clone(&object_store),
        "",
        Duration::from_millis(1),
        None,
    ));
    let artifact_store = ArtifactStore::new(object_store, "artifacts");
    (store, artifact_store)
}

/// Append the run lifecycle events up to (and including) `RunRunning`.
/// `completed` appends the terminal `WorkflowRunCompleted` event instead.
async fn append_run(store: &Database, run_id: &RunId, completed: bool) {
    let run_store = store.create_run(run_id).await.expect("create run store");
    workflow_event::append_event(&run_store, run_id, &workflow_event::Event::RunCreated {
        run_id:              *run_id,
        title:               None,
        settings:            serde_json::to_value(WorkflowSettings::default())
            .expect("workflow settings should serialize"),
        graph:               serde_json::to_value(Graph::new("test"))
            .expect("graph should serialize"),
        workflow_source:     None,
        labels:              std::collections::BTreeMap::default(),
        source_directory:    None,
        workflow_slug:       None,
        workflow_version_id: None,
        target:              None,
        automation:          None,
        provenance:          test_support::test_run_provenance(),
        manifest_blob:       None,
        spec_blob:           None,
        git:                 None,
        fork_source_ref:     None,
        retried_from:        None,
        parent_id:           None,
        web_url:             None,
    })
    .await
    .expect("append RunCreated");
    workflow_event::append_event(&run_store, run_id, &workflow_event::Event::RunRunnable {
        source: fabro_types::RunRunnableSource::StartRequested,
        actor:  None,
    })
    .await
    .expect("append RunRunnable");
    workflow_event::append_event(&run_store, run_id, &workflow_event::Event::RunStarting)
        .await
        .expect("append RunStarting");
    workflow_event::append_event(
        &run_store,
        run_id,
        &workflow_event::Event::WorkflowRunStarted {
            name:         "test".to_string(),
            run_id:       *run_id,
            base_branch:  Some("main".to_string()),
            base_sha:     Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string()),
            run_branch:   Some("fabro/run/test".to_string()),
            worktree_dir: None,
            goal:         Some("Test run wait".to_string()),
        },
    )
    .await
    .expect("append WorkflowRunStarted");
    workflow_event::append_event(&run_store, run_id, &workflow_event::Event::RunRunning)
        .await
        .expect("append RunRunning");
    if completed {
        workflow_event::append_event(
            &run_store,
            run_id,
            &workflow_event::Event::WorkflowRunCompleted {
                timing:               fabro_types::RunTiming::wall_only(1),
                artifact_count:       0,
                status:               "succeeded".to_string(),
                reason:               SuccessReason::Completed,
                failure:              None,
                total_usd_micros:     None,
                final_git_commit_sha: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
                final_patch:          Some(String::new()),
                diff_summary:         None,
                billing:              None,
            },
        )
        .await
        .expect("append WorkflowRunCompleted");
    }
}

async fn wait_app() -> (axum::Router, Arc<Database>) {
    let settings = test_settings();
    let (store, artifact_store) = store_bundle();
    let state = test_app_state_with_store(
        settings.server_settings,
        settings.manifest_run_defaults,
        5,
        Arc::clone(&store),
        artifact_store,
    );
    let app = fabro_server::test_support::build_test_router(state);
    (app, store)
}

#[tokio::test]
async fn terminal_run_returns_terminal_immediately() {
    let (app, store) = wait_app().await;
    let run_id = RunId::new();
    append_run(&store, &run_id, true).await;
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
    let (app, store) = wait_app().await;
    let run_id = RunId::new();
    append_run(&store, &run_id, false).await;
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
    let (app, store) = wait_app().await;
    let run_id = RunId::new();
    append_run(&store, &run_id, false).await;
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
async fn unknown_until_value_returns_400() {
    let (app, _store) = wait_app().await;
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
