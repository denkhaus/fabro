use super::*;

#[tokio::test]
async fn create_completion_missing_messages_returns_422() {
    let app = test_app_with();

    let req = Request::builder()
        .method("POST")
        .uri(api("/completions"))
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::UNPROCESSABLE_ENTITY).await;
}

#[tokio::test]
async fn create_completion_invalid_reasoning_effort_returns_422() {
    let app = test_app_with();

    let req = Request::builder()
        .method("POST")
        .uri(api("/completions"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "messages": [],
                "reasoning_effort": "bogus"
            })
            .to_string(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::UNPROCESSABLE_ENTITY).await;
}

#[tokio::test]
async fn create_completion_unknown_provider_returns_clear_error() {
    let app = test_app_with();

    let req = Request::builder()
        .method("POST")
        .uri(api("/completions"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "provider": "missing-provider",
                "model": "gpt-5.4",
                "stream": false,
                "messages": [
                    {
                        "role": "user",
                        "content": [{"type": "text", "text": "hi"}]
                    }
                ]
            })
            .to_string(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::BAD_REQUEST).await;
    assert_eq!(
        body["errors"][0]["detail"],
        "unknown model provider 'missing-provider'"
    );
}

#[tokio::test]
async fn create_completion_unsupported_reasoning_efforts_return_bad_request() {
    let upstream = MockServer::start();
    let completion = upstream.mock(|when, then| {
        when.method(POST);
        then.status(500);
    });
    let state = TestAppStateBuilder::new()
        .provider_base_url("moonshot", upstream.url("/v1"))
        .vault_entries([(EnvVars::KIMI_API_KEY, "test-kimi-api-key")])
        .build();
    let app = crate::test_support::build_test_router(state);

    for stream in [false, true] {
        for effort in ["medium", "xhigh"] {
            let req = Request::builder()
                .method("POST")
                .uri(api("/completions"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "provider": "moonshot",
                        "model": "kimi-k3",
                        "reasoning_effort": effort,
                        "stream": stream,
                        "messages": [
                            {
                                "role": "user",
                                "content": [{"type": "text", "text": "hi"}]
                            }
                        ]
                    })
                    .to_string(),
                ))
                .unwrap();

            let response = app.clone().oneshot(req).await.unwrap();
            let body = response_json!(response, StatusCode::BAD_REQUEST).await;
            assert_eq!(
                body["errors"][0]["detail"], "model moonshot/kimi-k3 does not support reasoning",
                "stream={stream} effort={effort}"
            );
        }
    }

    completion.assert_calls(0);
}

#[tokio::test]
async fn create_completion_returns_disjoint_usage_buckets() {
    let upstream = MockServer::start();
    let completion = upstream.mock(|when, then| {
        when.method(POST).path("/v1/chat/completions");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "id": "chatcmpl-usage",
                "model": "kimi-k3",
                "choices": [{
                    "message": {"role": "assistant", "content": "OK"},
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 200,
                    "completion_tokens": 30,
                    "total_tokens": 230,
                    "prompt_tokens_details": {
                        "cached_tokens": 50,
                        "cache_write_tokens": 100
                    },
                    "completion_tokens_details": {
                        "reasoning_tokens": 20
                    }
                }
            }));
    });
    let state = TestAppStateBuilder::new()
        .provider_base_url("moonshot", upstream.base_url())
        .vault_entries([(EnvVars::KIMI_API_KEY, "test-kimi-api-key")])
        .build();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/completions"))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "provider": "moonshot",
                "model": "kimi-k3",
                "stream": false,
                "messages": [{
                    "role": "user",
                    "content": [{"type": "text", "text": "hi"}]
                }]
            })
            .to_string(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(
        body["usage"],
        json!({
            "input": 50,
            "output": 10,
            "reasoning": 20,
            "cache_read": 50,
            "cache_write": 100
        })
    );
    completion.assert();
}

#[tokio::test]
async fn create_completion_default_model_uses_app_state_catalog() {
    let upstream = MockServer::start();
    let completion = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .json_body_includes(r#"{"model":"acme-large"}"#);
        then.status(500)
            .header("content-type", "application/json")
            .json_body(json!({"error": {"message": "expected test failure"}}));
    });
    let overlay = acme_overlay(&upstream.base_url());
    let state = TestAppStateBuilder::new()
        .llm_overlay_toml(&overlay)
        .vault_entries([("ACME_API_KEY", "acme-test-key")])
        .build();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/completions"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "stream": false,
                "messages": [
                    {
                        "role": "user",
                        "content": [{"type": "text", "text": "hi"}]
                    }
                ]
            })
            .to_string(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::BAD_GATEWAY).await;
    assert!(
        body["errors"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("expected test failure"),
        "unexpected error body: {body:?}"
    );
    assert!(completion.calls() >= 1);
}

#[tokio::test]
async fn create_completion_structured_output_forwards_reasoning_effort() {
    let upstream = MockServer::start();
    let completion = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .json_body_includes(r#"{"model":"kimi-k3","reasoning_effort":"high"}"#);
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "id": "chatcmpl-kimi-structured",
                "model": "kimi-k3",
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": "{\"answer\":42}"
                    },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 10,
                    "completion_tokens": 4,
                    "total_tokens": 14
                }
            }));
    });
    let state = TestAppStateBuilder::new()
        .provider_base_url("moonshot", upstream.base_url())
        .vault_entries([(EnvVars::KIMI_API_KEY, "test-kimi-api-key")])
        .build();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/completions"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "provider": "moonshot",
                "model": "kimi-k3",
                "reasoning_effort": "high",
                "stream": false,
                "schema": {
                    "type": "object",
                    "properties": {
                        "answer": {"type": "integer"}
                    },
                    "required": ["answer"]
                },
                "messages": [
                    {
                        "role": "user",
                        "content": [{"type": "text", "text": "Return the answer."}]
                    }
                ]
            })
            .to_string(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["output"], json!({"answer": 42}));
    completion.assert();
}
