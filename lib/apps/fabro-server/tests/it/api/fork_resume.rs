//! Fork-only resume conformance pin (fabro-7627, salvaged from run
//! 01M2J1BA2JC4S6SA10YEJHHMVP 2026-09-15). The POST /runs/{id}/resume
//! endpoint is served by `server/handler/fork_resume.rs` (fork-only);
//! this file pins its wire behavior where upstream merges cannot drop
//! it. Registered via a one-line `mod fork_resume;` in `api/mod.rs`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;

use crate::helpers::{MINIMAL_DOT, api, minimal_intent_json, response_json, settings_from_toml};

async fn create_run(app: &axum::Router, intent: serde_json::Value) -> serde_json::Value {
    let request = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&intent).expect("intent should serialize"),
        ))
        .expect("create run request should build");
    let response = app.clone().oneshot(request).await.expect("router serves");
    response_json(response, StatusCode::CREATED, "create run").await
}

#[tokio::test]
async fn fork_resume_endpoint_rejects_non_terminal_run_with_conflict() {
    let workspace = tempfile::tempdir().expect("run target workspace should be created");
    let settings = settings_from_toml(
        r"
_version = 1
",
    );
    let state = fabro_server::test_support::TestAppStateBuilder::new()
        .runtime_settings(settings.server_settings, settings.manifest_run_defaults)
        .vault_entries([("OPENAI_API_KEY", "test-key")])
        .build();
    let app = fabro_server::test_support::build_test_router(state);
    let created = create_run(
        &app,
        minimal_intent_json(&app, MINIMAL_DOT, workspace.path()).await,
    )
    .await;
    let run_id = created["id"].as_str().unwrap();

    let request = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/resume")))
        .body(Body::empty())
        .expect("resume request should build");
    let response = app.clone().oneshot(request).await.unwrap();
    let body = response_json(response, StatusCode::CONFLICT, "POST resume").await;
    assert!(
        body["errors"][0]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("must be terminal")),
        "expected a must-be-terminal conflict, got: {body}"
    );
}
