use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fabro_server::server::{AppState, RouterOptions, build_router_with_options};
use fabro_server::test_support::test_auth_mode;
use tower::ServiceExt;

use crate::helpers::test_settings;

fn app(state: Arc<AppState>) -> axum::Router {
    build_router_with_options(state, &test_auth_mode(), RouterOptions::default())
}

/// The ask session's tracker read (fabro-d6de): a worker token with the
/// `agent:run_tools` capability may list the seeds, a plain run worker may
/// not, and an unauthenticated caller never may.
#[tokio::test]
async fn seeds_listing_accepts_run_tools_workers_only() {
    let settings = test_settings();
    let state = fabro_server::test_support::TestAppStateBuilder::new()
        .runtime_settings(settings.server_settings, settings.manifest_run_defaults)
        .build();
    let run_id = fabro_types::RunId::new();
    let app = app(state.clone());

    let tools_token = state.test_issue_worker_token_with_run_tools(&run_id);
    // This app configures no seeds source, so an AUTHORIZED caller gets
    // 503 (`seeds_source_unconfigured`), not 401: the seam under test is
    // the auth boundary, and the run_tools worker passes it.
    let ok = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/seeds")
                .header("authorization", format!("Bearer {tools_token}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(ok.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        ok.headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("application/json")),
        "the 503 names the unconfigured source, not an auth refusal"
    );

    let plain_token = state.test_issue_worker_token(&run_id);
    let refused = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/seeds")
                .header("authorization", format!("Bearer {plain_token}"))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    // A plain run worker authenticates but lacks the capability: 403,
    // distinct from the anonymous 401.
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);

    let anon = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/seeds")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);
}
