//! HTTP-level integration tests for `GET /api/v1/runs/{id}/wait`
//! (fabro-571e).
//!
//! These tests exercise the long-poll handler's observable contracts:
//! immediate `terminal` results for finished runs, structured `timeout`
//! results carrying the current status, the `until=merged` pull-request
//! precondition, and query validation. GitHub-backed merge detection is
//! covered against a local `httpmock` GitHub API — the tests never reach
//! the real network.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fabro_server::test_support::{
    TestAppStateBuilder, build_test_router, test_app_state_with_store,
};
use fabro_store::{ArtifactStore, Database};
use fabro_types::{Graph, PullRequestLink, RunId, WorkflowSettings, test_support};
use fabro_workflow::event as workflow_event;
use fabro_workflow::run_status::SuccessReason;
use httpmock::MockServer;
use object_store::memory::InMemory as MemoryObjectStore;
use serde_json::json;
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

fn wait_app() -> (axum::Router, Arc<Database>) {
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
    let (app, store) = wait_app();
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
    let (app, store) = wait_app();
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
    let (app, store) = wait_app();
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
async fn out_of_range_timeout_ms_returns_400() {
    let (app, _store) = wait_app();
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
    let (app, _store) = wait_app();
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

// ── until=merged blocked-gate regression tests (fabro-bde4) ──────────────

/// Open PR payload served by the mock GitHub API, parameterized only by
/// `mergeable_state`.
fn open_pr_body(mergeable_state: &str) -> String {
    json!({
        "number": 42,
        "title": "Develop child",
        "body": null,
        "state": "open",
        "draft": false,
        "merged": false,
        "merged_at": null,
        "mergeable": false,
        "mergeable_state": mergeable_state,
        "additions": 10,
        "deletions": 3,
        "changed_files": 2,
        "html_url": "https://github.com/acme/widgets/pull/42",
        "user": { "login": "testuser" },
        "head": { "ref": "fabro/run/child" },
        "base": { "ref": "main" },
        "created_at": "2026-09-07T08:00:00Z",
        "updated_at": "2026-09-07T09:00:00Z"
    })
    .to_string()
}

/// App whose GitHub API client points at `github_base_url`, with token
/// credentials and an in-memory store, mirroring the mock-GitHub pattern
/// from the fabro-server unit suite.
fn github_wait_app(github_base_url: String) -> (axum::Router, Arc<Database>) {
    let settings = crate::helpers::settings_from_toml(
        r#"
_version = 1

[server.integrations.github]
strategy = "token"
"#,
    );
    let (store, artifact_store) = store_bundle();
    let state = TestAppStateBuilder::new()
        .runtime_settings(settings.server_settings, settings.manifest_run_defaults)
        .store_bundle(Arc::clone(&store), artifact_store)
        .vault_entries([("GITHUB_TOKEN", "ghu_test")])
        .github_api_base_url(github_base_url)
        .build();
    (build_test_router(state), store)
}

/// A running run with a linked acme/widgets#42 pull request.
async fn append_running_run_with_pr(store: &Database, run_id: &RunId) {
    append_run(store, run_id, false).await;
    let run_store = store.open_run(run_id).await.expect("open run store");
    workflow_event::append_event(
        &run_store,
        run_id,
        &workflow_event::Event::PullRequestLinked {
            pull_request: PullRequestLink {
                owner:  "acme".to_string(),
                repo:   "widgets".to_string(),
                number: 42,
            },
        },
    )
    .await
    .expect("append PullRequestLinked");
}

/// An open PR whose mergeable_state stays blocked across consecutive
/// polls must return `reached=blocked` (with the PR link attached), not
/// loop `timeout` forever — the fabro-bde4 incident.
#[tokio::test]
async fn merged_wait_reports_blocked_when_gate_stuck() {
    let github = MockServer::start();
    let pr_mock = github.mock(|when, then| {
        when.method("GET")
            .path("/repos/acme/widgets/pulls/42")
            .header("authorization", "Bearer ghu_test");
        then.status(200)
            .header("content-type", "application/json")
            .body(open_pr_body("blocked"));
    });
    let (app, store) = github_wait_app(github.base_url());
    let run_id = RunId::new();
    append_running_run_with_pr(&store, &run_id).await;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/wait?until=merged&timeout_ms=10000"
        )))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let body = response_json(
        resp,
        StatusCode::OK,
        "GET /api/v1/runs/{id}/wait blocked gate",
    )
    .await;

    assert_eq!(body["reached"].as_str(), Some("blocked"));
    assert_eq!(body["pull_request"]["owner"].as_str(), Some("acme"));
    assert_eq!(body["pull_request"]["number"].as_u64(), Some(42));
    // The sustained window needs two consecutive blocked polls.
    assert!(pr_mock.calls_async().await >= 2);
}

/// A single transient blocked observation (GitHub still computing, then
/// reporting unknown) must NOT fire `blocked`; the wait keeps going and
/// ends in a structured timeout.
#[tokio::test]
async fn merged_wait_ignores_transient_blocked_observation() {
    let github = MockServer::start();
    let mut blocked_mock = github.mock(|when, then| {
        when.method("GET")
            .path("/repos/acme/widgets/pulls/42")
            .header("authorization", "Bearer ghu_test");
        then.status(200)
            .header("content-type", "application/json")
            .body(open_pr_body("blocked"));
    });
    let (app, store) = github_wait_app(github.base_url());
    let run_id = RunId::new();
    append_running_run_with_pr(&store, &run_id).await;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/wait?until=merged&timeout_ms=4000"
        )))
        .body(Body::empty())
        .unwrap();
    let task = tokio::spawn(app.oneshot(req));
    // After the first (blocked) poll lands, swap the PR to mergeable_state
    // unknown — GitHub recompute — so the second poll breaks the streak.
    for _ in 0..500 {
        if blocked_mock.calls_async().await >= 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    blocked_mock.delete();
    github.mock(|when, then| {
        when.method("GET")
            .path("/repos/acme/widgets/pulls/42")
            .header("authorization", "Bearer ghu_test");
        then.status(200)
            .header("content-type", "application/json")
            .body(open_pr_body("unknown"));
    });

    let resp = task.await.unwrap().unwrap();
    let body = response_json(
        resp,
        StatusCode::OK,
        "GET /api/v1/runs/{id}/wait transient blocked",
    )
    .await;

    assert_eq!(body["reached"].as_str(), Some("timeout"));
    assert_eq!(body["status"]["kind"].as_str(), Some("running"));
}
