//! Cursor-pagination tests for the per-stage events endpoint (demo mode).
//!
//! The stage-events route uses `since_seq=` + `limit=` (cursor-based) instead
//! of the offset-based `page[limit]/page[offset]` pagination used by other
//! list endpoints, so it gets its own test rather than living in the generic
//! offset-shape matrix.

#![allow(
    clippy::absolute_paths,
    reason = "This test module prefers explicit type paths over extra imports."
)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use super::helpers::{response_json, test_app_state};

async fn get_json(app: &axum::Router, uri: &str) -> serde_json::Value {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .header("x-fabro-demo", "1")
        .body(Body::empty())
        .expect("event pagination request should build");
    let response = app.clone().oneshot(req).await.unwrap();
    response_json(response, StatusCode::OK, format!("GET {uri}")).await
}

#[tokio::test]
async fn demo_stage_events_default_returns_all_fixture_events_with_no_more() {
    let app = fabro_server::test_support::build_test_router(test_app_state());

    let body = get_json(&app, "/api/v1/runs/run-1/stages/detect-drift@1/events").await;
    let data = body["data"].as_array().expect("data is an array");

    assert_eq!(data.len(), 24, "every fixture event should be returned");
    assert_eq!(body["meta"]["has_more"], false);
}

#[tokio::test]
async fn demo_stage_events_limit_one_signals_has_more() {
    let app = fabro_server::test_support::build_test_router(test_app_state());

    let body = get_json(
        &app,
        "/api/v1/runs/run-1/stages/detect-drift@1/events?limit=1",
    )
    .await;
    let data = body["data"].as_array().expect("data is an array");

    assert_eq!(data.len(), 1);
    assert_eq!(body["meta"]["has_more"], true);
}

#[tokio::test]
async fn demo_stage_events_since_seq_filters_out_earlier_events() {
    let app = fabro_server::test_support::build_test_router(test_app_state());

    // The fixture seqs are 1..=24. since_seq=4 should skip the first three.
    let body = get_json(
        &app,
        "/api/v1/runs/run-1/stages/detect-drift@1/events?since_seq=4",
    )
    .await;
    let data = body["data"].as_array().expect("data is an array");

    assert_eq!(data.len(), 21);
    let seqs: Vec<u64> = data
        .iter()
        .map(|envelope| envelope["seq"].as_u64().expect("seq is a number"))
        .collect();
    assert_eq!(seqs, (4..=24).collect::<Vec<u64>>());
    assert_eq!(body["meta"]["has_more"], false);
}

#[tokio::test]
async fn demo_run_state_carries_the_agent_stages_fold() {
    let app = fabro_server::test_support::build_test_router(test_app_state());

    let body = get_json(&app, "/api/v1/runs/run-1/state").await;
    let stage = &body["stages"]["detect-drift@1"];
    assert_eq!(stage["handler"], "agent");
    let agent = &stage["agent"];
    assert_eq!(agent["root_session_id"], "ses_demo_detect_drift");
    assert_eq!(
        agent["route"]["model"], "gpt-5.4",
        "the route after the failover"
    );
    assert_eq!(agent["activity"], "idle");
    assert_eq!(
        agent["mcp_servers"]["github"]["tools"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(agent["mcp_servers"]["github"]["invoked"], true);
    assert_eq!(
        agent["mcp_servers"]["atlassian"]["error"],
        "auth failed: the API token has expired"
    );
    assert_eq!(agent["skills"]["activated"][0]["name"], "drift-triage");
    assert_eq!(agent["subagents"][0]["status"]["status"], "completed");
    assert_eq!(agent["failovers"][0]["to"], "openai/gpt-5.4");
    assert_eq!(agent["compactions"].as_array().unwrap().len(), 1);
    assert_eq!(
        agent["files_touched"],
        serde_json::json!(["reports/drift.md"])
    );
    assert!(agent.get("pending_writes").is_none());
    assert!(body["stages"]["apply-changes@2"]["agent"].is_null());
}
