//! The PR state chip's other two writers over a mocked GitHub
//! (denkhaus/work#1173): the close door records `pull_request.closed`
//! after GitHub accepts the close, and the merged-wait (the supervisor
//! arm) records `pull_request.merged` / `pull_request.closed` for
//! outcomes that landed OUTSIDE Fabro — an external merge or a manual
//! close. The merge door's own recording is pinned in
//! `merge_door_tests.rs`.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use fabro_store::platform_records::PlatformRecordKind;
use fabro_types::{PullRequestLink, PullRequestState, RunId};
use httpmock::MockServer;
use serde_json::json;
use tower::ServiceExt;

use super::merge_door_tests::seed_run;
use super::run_records;
use super::tests::create_github_token_app_state;
use crate::server::AppState;
use crate::test_support::build_test_router;

fn pull_request_link() -> PullRequestLink {
    PullRequestLink {
        owner:  "denkhaus".to_string(),
        repo:   "fabro".to_string(),
        number: 501,
        state:  None,
    }
}

async fn call(
    state: &Arc<AppState>,
    worker: &RunId,
    method: Method,
    path_and_query: &str,
) -> axum::http::Response<Body> {
    let app = build_test_router(Arc::clone(state));
    let token = state.test_issue_worker_token_with_run_tools(worker);
    app.oneshot(
        Request::builder()
            .method(method)
            .uri(format!("/api/v1/runs/{path_and_query}"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .expect("request builds"),
    )
    .await
    .expect("request runs")
}

async fn records_of(state: &Arc<AppState>, run: &RunId, kind: PlatformRecordKind) -> usize {
    state
        .stores
        .run_summaries
        .platform_records()
        .read_kind(run, kind)
        .await
        .expect("the records read")
        .len()
}

async fn folded_pr_state(state: &Arc<AppState>, run: &RunId) -> Option<PullRequestState> {
    run_records::projection(state, *run)
        .await
        .expect("the projection reads")
        .expect("the run's projection exists")
        .pull_request
        .as_ref()
        .and_then(|link| link.state)
}

/// The close door: a GitHub-accepted close records exactly one
/// `pull_request.closed` and the folded link settles `closed`.
#[tokio::test]
async fn the_close_door_records_the_closed_state() {
    let github = MockServer::start();
    let state = create_github_token_app_state(Some("ghu_test"), Some(github.base_url()));
    let target = RunId::new();
    seed_run(&state, &target, "develop", Some(pull_request_link())).await;

    let close_mock = github.mock(|when, then| {
        when.method("PATCH")
            .path("/repos/denkhaus/fabro/pulls/501")
            .header("authorization", "Bearer ghu_test")
            .json_body(json!({"state": "closed"}));
        then.status(200).json_body(json!({"number": 501}));
    });

    let response = call(
        &state,
        &target,
        Method::POST,
        &format!("{target}/pull_request/close"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "the close door closes");
    assert_eq!(close_mock.calls(), 1, "exactly one GitHub close call");
    assert_eq!(
        records_of(&state, &target, PlatformRecordKind::PullRequestClosed).await,
        1,
        "the close door records the closed outcome exactly once"
    );
    assert_eq!(
        folded_pr_state(&state, &target).await,
        Some(PullRequestState::Closed),
        "the folded link carries the closed state for the run list"
    );
}

/// The supervisor arm: the merged-wait observes an EXTERNAL merge (no
/// Fabro door ran) and records `pull_request.merged` — the run list
/// colours the chip even though the merge happened on GitHub itself.
#[tokio::test]
async fn the_merged_wait_records_an_externally_merged_pull_request() {
    let github = MockServer::start();
    let state = create_github_token_app_state(Some("ghu_test"), Some(github.base_url()));
    let target = RunId::new();
    seed_run(&state, &target, "develop", Some(pull_request_link())).await;

    github.mock(|when, then| {
        when.method("GET")
            .path("/repos/denkhaus/fabro/pulls/501")
            .header("authorization", "Bearer ghu_test");
        then.status(200).json_body(json!({
            "number": 501,
            "title": "merged elsewhere",
            "state": "closed",
            "draft": false,
            "merged": true,
            "merged_at": "2026-11-08T09:00:00Z",
            "mergeable": true,
            "additions": 3,
            "deletions": 1,
            "changed_files": 1,
            "html_url": "https://github.com/denkhaus/fabro/pull/501",
            "user": {"login": "a-human"},
            "head": {"ref": "fabro/run/01TARGET"},
            "base": {"ref": "denkhaus"},
            "created_at": "2026-11-08T08:00:00Z",
            "updated_at": "2026-11-08T09:00:00Z"
        }));
    });

    let response = call(
        &state,
        &target,
        Method::GET,
        &format!("{target}/wait?until=merged&timeout_ms=5000"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "the merged wait returns");
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("wait body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("wait json");
    assert_eq!(body["reached"], json!("merged"), "the wait reports merged");
    assert_eq!(
        records_of(&state, &target, PlatformRecordKind::PullRequestMerged).await,
        1,
        "the wait records the observed merged outcome exactly once"
    );
    assert_eq!(
        folded_pr_state(&state, &target).await,
        Some(PullRequestState::Merged),
        "the folded link carries the merged state for the run list"
    );
}
