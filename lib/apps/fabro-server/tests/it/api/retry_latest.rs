//! Retry-on-latest refusal paths (work#1320): the door's guards, fast
//! and engine-free. The green path (faulty version → fix → re-drive →
//! successor green with carried work) is the bench facet in fabro-test
//! — the item's closes-when proof.

use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::helpers::{api, test_app_with_no_providers};

async fn retry_latest_status(app: &axum::Router, run_id: &str) -> (StatusCode, String) {
    let request = Request::builder()
        .method(axum::http::Method::POST)
        .uri(api(&format!("/runs/{run_id}/retry-latest")))
        .body(axum::body::Body::empty())
        .expect("retry-latest request should build");
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("call should answer");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body should collect")
        .to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

#[tokio::test]
async fn an_unknown_run_is_not_found() {
    let app = test_app_with_no_providers();
    // A valid ULID-shaped id that exists in no store (Crockford base32
    // alphabet, ULID length): the parse passes, the lookup refuses.
    let (status, body) = retry_latest_status(&app, "01M7CRT3ST0000000000000000").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn a_store_unknown_run_refuses_without_a_successor() {
    let app = test_app_with_no_providers();
    let (status, body) = retry_latest_status(&app, "01M7CRT3ST0000000000000001").await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::CONFLICT,
        "expected a refusal, got {status}: {body}"
    );
}
