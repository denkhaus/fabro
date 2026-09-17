use super::*;

#[tokio::test]
async fn run_tools_worker_cannot_call_user_only_non_mcp_routes() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let origin_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let target_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_run_tools_worker_token(&origin_run_id);

    for (method, path) in [
        (Method::POST, format!("/runs/{target_run_id}/approve")),
        (Method::POST, format!("/runs/{target_run_id}/deny")),
        (Method::GET, format!("/runs/{target_run_id}/timeline")),
    ] {
        let response = app
            .clone()
            .oneshot(bearer_request(
                method.clone(),
                &path,
                &worker_token,
                Body::empty(),
            ))
            .await
            .unwrap();
        assert!(
            matches!(
                response.status(),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            ),
            "{method} {path} unexpectedly accepted run-tools worker token with status {}",
            response.status()
        );
    }
}
