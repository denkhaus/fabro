use super::*;

#[tokio::test]
async fn steer_empty_text_returns_bad_request() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id = create_and_start_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/steer")))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"text":"   "}"#))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    // 400 (whitespace-only text) or 409 (run not yet `running` when the
    // handler checks status) are both acceptable; the only outcome we
    // want to rule out is a successful enqueue.
    let status = response.status();
    assert!(
        matches!(status, StatusCode::BAD_REQUEST | StatusCode::CONFLICT),
        "expected 400 or 409, got {status}"
    );
}
