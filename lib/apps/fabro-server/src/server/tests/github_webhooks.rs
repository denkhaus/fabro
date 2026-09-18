use super::*;

#[tokio::test]
async fn github_webhook_rejects_missing_signature() {
    let app = webhook_test_app(crate::test_support::test_auth_mode());
    let body = br#"{"action":"opened"}"#;

    let response = app
        .oneshot(webhook_request(None, None, body))
        .await
        .unwrap();
    assert_status!(response, StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn github_webhook_rejects_signature_signed_with_wrong_secret() {
    let app = webhook_test_app(crate::test_support::test_auth_mode());
    let body = br#"{"action":"opened"}"#;
    let bad_signature = compute_signature(b"wrong-secret", body);

    let response = app
        .oneshot(webhook_request(Some(&bad_signature), None, body))
        .await
        .unwrap();
    assert_status!(response, StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn github_webhook_accepts_valid_signature_when_auth_disabled() {
    let body = br#"{"repository":{"full_name":"owner/repo"},"action":"opened"}"#;
    let signature = compute_signature(TEST_WEBHOOK_SECRET.as_bytes(), body);
    let app = webhook_test_app(crate::test_support::test_auth_mode());

    let response = app
        .oneshot(webhook_request(Some(&signature), None, body))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;
}

#[tokio::test]
async fn github_webhook_accepts_valid_signature_without_bearer_token() {
    let body = br#"{"repository":{"full_name":"owner/repo"},"action":"opened"}"#;
    let signature = compute_signature(TEST_WEBHOOK_SECRET.as_bytes(), body);
    let app = webhook_test_app(dev_token_auth_mode());

    let response = app
        .oneshot(webhook_request(Some(&signature), None, body))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;
}

#[tokio::test]
async fn github_webhook_accepts_valid_signature_with_wrong_bearer_token() {
    let body = br#"{"repository":{"full_name":"owner/repo"},"action":"opened"}"#;
    let signature = compute_signature(TEST_WEBHOOK_SECRET.as_bytes(), body);
    let app = webhook_test_app(dev_token_auth_mode());

    let response = app
        .oneshot(webhook_request(
            Some(&signature),
            Some(&format!("Bearer {WRONG_DEV_TOKEN}")),
            body,
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;
}
