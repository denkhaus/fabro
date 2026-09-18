use super::*;

#[tokio::test]
async fn resolve_llm_client_ignores_env_lookup_provider_tokens() {
    let state = test_app_state_with_env_lookup(
        default_test_server_settings(),
        RunLayer::default(),
        5,
        |name| (name == EnvVars::OPENAI_API_KEY).then(|| "env-openai-key".to_string()),
    );

    let llm_result = state.resolve_llm_client().await.unwrap();

    assert!(
        llm_result.provider_ids().is_empty(),
        "server LLM credentials should come from vault only"
    );
    assert!(llm_result.auth_issues.is_empty());
}

#[tokio::test]
async fn resolve_llm_client_from_source_with_no_credentials_has_no_ready_providers() {
    let catalog = state_test_catalog();
    let built = resolve_llm_client_from_source(Arc::new(FailingCredentialSource), catalog, None)
        .await
        .expect("a client with no credentials still builds");

    assert!(built.ready.is_empty());
    assert!(built.auth_issues.is_empty());
    assert!(built.provider_ids().is_empty());
}

#[tokio::test]
async fn llm_source_configured_providers_reads_openai_token_from_vault() {
    let state = test_app_state_with_env_lookup(
        default_test_server_settings(),
        RunLayer::default(),
        5,
        |_| None,
    );
    state
        .stores
        .vault
        .set(
            "OPENAI_API_KEY",
            "vault-openai-key",
            SecretType::Token,
            None,
        )
        .await
        .unwrap();

    assert_eq!(state.configured_llm_provider_ids().await, vec![
        lithos_llm::catalog::builtin::openai()
    ]);
}

#[tokio::test]
async fn create_run_without_ready_llm_provider_rejects_implicit_model_selection() {
    let state = TestAppStateBuilder::new().env_lookup(|_| None).build();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api("/runs"))
                .header("content-type", "application/json")
                .body(Body::from(test_intent(&app, MINIMAL_DOT).await.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::UNPROCESSABLE_ENTITY).await;

    assert!(
        body["errors"][0]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("no default model is available")),
        "unexpected response: {body}"
    );
    assert!(state.runs.lock().expect("runs lock poisoned").is_empty());
}

#[tokio::test]
async fn post_runs_run_intent_dry_run_uses_configured_target_provider() {
    let folder = tempfile::tempdir().unwrap();
    let folder_path = folder
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let cases = [
        (
            test_app_state(),
            None,
            json!({
                "kind": "git",
                "repo": "fabro-sh/fabro",
                "branch": "main"
            }),
            json!({ "dry_run": true }),
        ),
        (
            test_app_state(),
            None,
            json!({ "kind": "none" }),
            json!({ "dry_run": true }),
        ),
        (
            TestAppStateBuilder::new()
                .default_environment_provider(Some(SandboxProviderKind::DAYTONA))
                .vault_entries([(fabro_static::EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
                .build(),
            Some("_version = 1\n[run.execution]\nmode = \"dry_run\"\n"),
            json!({ "kind": "none" }),
            json!({}),
        ),
        (
            TestAppStateBuilder::new()
                .default_environment_provider(Some(SandboxProviderKind::DAYTONA))
                .vault_entries([(fabro_static::EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
                .build(),
            Some("_version = 1\n[run.execution]\nmode = \"dry_run\"\n"),
            json!({
                "kind": "git",
                "repo": "fabro-sh/fabro",
                "branch": "main"
            }),
            json!({}),
        ),
        (
            TestAppStateBuilder::new()
                .runtime_settings(
                    default_test_server_settings(),
                    manifest_run_defaults_from_toml("[run.execution]\nmode = \"dry_run\"\n"),
                )
                .default_environment_provider(Some(SandboxProviderKind::LOCAL))
                .vault_entries([(fabro_static::EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
                .build(),
            None,
            json!({ "kind": "folder", "path": folder_path }),
            json!({}),
        ),
    ];

    for (state, workflow_toml, target, args) in cases {
        let app = crate::test_support::build_test_router(Arc::clone(&state));
        let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, workflow_toml).await;
        let body = post_run_intent(
            &app,
            json!({
                "workflow_version_id": workflow_version_id,
                "target": target,
                "args": args
            }),
        )
        .await;
        let run_id = body["id"].as_str().unwrap().parse::<RunId>().unwrap();
        let projection = state
            .stores
            .runs
            .open_run_reader(&run_id)
            .await
            .unwrap()
            .state()
            .await
            .unwrap();

        assert_eq!(projection.spec.settings.run.execution.mode, RunMode::DryRun);
        assert_eq!(
            serde_json::to_value(projection.spec.target.unwrap()).unwrap(),
            target
        );
    }
}

#[tokio::test]
async fn validate_endpoint_uses_app_state_catalog_for_model_diagnostics() {
    let state = TestAppStateBuilder::new()
        .llm_overlay_toml(&acme_overlay("https://api.acme.test/v1"))
        .build();
    let app = crate::test_support::build_test_router(state);
    let dot = r#"digraph Test {
        graph [goal="Test"]
        start [shape=Mdiamond]
        work [model="acme-large", provider="acme", prompt="Do it"]
        exit  [shape=Msquare]
        start -> work -> exit
    }"#;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api("/validate"))
                .header("content-type", "application/json")
                .body(manifest_body(dot))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let diagnostics = body["workflow"]["diagnostics"].as_array().unwrap();

    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| diagnostic["rule"] != "node_model_known"),
        "custom model/provider should validate against app-state catalog: {body}"
    );
}

#[tokio::test]
async fn list_run_stages_includes_stage_model_usage() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();

    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ])
    .await;
    append_scoped_stage_event(
        &state,
        run_id,
        "prompt",
        1,
        &workflow_event::Event::StageStarted {
            graph_visit:           None,
            resumed_from_stage_id: None,
            node_id:               "prompt".to_string(),
            name:                  "Prompt".to_string(),
            index:                 0,
            handler_type:          "prompt".to_string(),
            attempt:               1,
            max_attempts:          1,
        },
    )
    .await;
    append_scoped_stage_event(
        &state,
        run_id,
        "prompt",
        1,
        &workflow_event::Event::Prompt {
            stage:            "prompt".to_string(),
            visit:            1,
            text:             "Summarize".to_string(),
            mode:             Some(StageModelUsage::MODE_PROMPT.to_string()),
            provider:         Some("openai".to_string()),
            model:            Some("gpt-5.5".to_string()),
            reasoning_effort: Some(ReasoningEffort::High),
            speed:            Some(Speed::Fast),
        },
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/stages")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(
        stage_entry(&body, "prompt@1")["provider_used"],
        json!({
            "mode": "prompt",
            "provider": "openai",
            "model": "gpt-5.5",
            "reasoning_effort": "high",
            "speed": "fast"
        })
    );
}

#[tokio::test]
async fn list_run_stages_reports_zero_usage_for_a_stage_that_called_no_model() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();

    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ])
    .await;
    let started = stage_started_event("script", "command");
    append_scoped_stage_event(&state, run_id, "script", 1, &started).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/stages")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;

    let usage = &stage_entry(&body, "script@1")["usage"];
    assert_eq!(usage["tokens"]["input"], 0);
    assert_eq!(usage["tokens"]["output"], 0);
    // No model ran, so there is nothing to price — not a $0.00 cost.
    assert!(usage.get("cost").is_none());
}

#[tokio::test]
async fn test_model_unknown_returns_404() {
    let app = test_app_with();

    let req = Request::builder()
        .method("POST")
        .uri(api("/models/nonexistent-model-xyz/test"))
        .header("content-type", "application/json")
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn test_model_explicit_provider_alias_returns_canonical_model_id_when_unavailable() {
    let state = test_app_state_with_env_lookup(
        default_test_server_settings(),
        RunLayer::default(),
        5,
        |_| None,
    );
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/models/sonnet/test?provider=anthropic"))
        .header("content-type", "application/json")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["model_id"], "claude-sonnet-5");
    assert_eq!(body["provider"], "anthropic");
    assert_eq!(body["status"], "skip");
}

#[tokio::test]
async fn test_model_unqualified_known_alias_requires_a_ready_provider() {
    let state = test_app_state_with_env_lookup(
        default_test_server_settings(),
        RunLayer::default(),
        5,
        |_| None,
    );
    let app = crate::test_support::build_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api("/models/sonnet/test"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_status!(response, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn model_api_keeps_duplicate_ids_provider_scoped_and_selects_ready_priority() {
    let direct_upstream = MockServer::start();
    let aggregator_upstream = MockServer::start();
    let direct_probe = direct_upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .json_body_includes(r#"{"model":"portable-model"}"#);
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "id": "chatcmpl-direct",
                "model": "portable-model",
                "choices": [{
                    "message": {"role": "assistant", "content": "OK"},
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 1,
                    "completion_tokens": 1,
                    "total_tokens": 2
                }
            }));
    });
    let aggregator_probe = aggregator_upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .json_body_includes(r#"{"model":"vendor/portable-model"}"#);
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "id": "chatcmpl-aggregator",
                "model": "vendor/portable-model",
                "choices": [{
                    "message": {"role": "assistant", "content": "OK"},
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 1,
                    "completion_tokens": 1,
                    "total_tokens": 2
                }
            }));
    });
    let overlay = format!(
        r#"
[providers.direct]
display_name = "Direct"
adapter = "openai-compatible"
codec = "openai-chat"
base_url = {direct}
auth = {{ type = "bearer" }}
priority = 120
default_model = "portable-model"

[providers.direct.metadata.agent]
profile = "openai"

[providers.direct.models.portable-model]
display_name = "Portable (direct)"
aliases = ["portable"]
api_model = "portable-model"
limits = {{ context_tokens = 1000, max_output_tokens = 500 }}
capabilities = {{ text = true }}

[providers.aggregator]
display_name = "Aggregator"
adapter = "openai-compatible"
codec = "openai-chat"
base_url = {aggregator}
auth = {{ type = "bearer" }}
priority = 110
default_model = "portable-model"

[providers.aggregator.metadata.agent]
profile = "openai"

[providers.aggregator.models.portable-model]
display_name = "Portable (aggregator)"
aliases = ["portable"]
api_model = "vendor/portable-model"
limits = {{ context_tokens = 1000, max_output_tokens = 500 }}
capabilities = {{ text = true }}
"#,
        direct = toml::Value::String(direct_upstream.base_url()),
        aggregator = toml::Value::String(aggregator_upstream.base_url()),
    );
    let state = TestAppStateBuilder::new()
        .llm_overlay_toml(&overlay)
        .vault_entries([
            ("DIRECT_API_KEY", "direct-test-key"),
            ("AGGREGATOR_API_KEY", "aggregator-test-key"),
        ])
        .build();
    let app = crate::test_support::build_test_router(state);

    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api("/models?query=portable-model"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let list = response_json!(list, StatusCode::OK).await;
    let rows = list["data"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows.iter()
            .map(|row| row["provider"].as_str().unwrap())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["aggregator", "direct"])
    );
    assert!(
        rows.iter()
            .all(|row| row["id"] == "portable-model" && row["configured"] == true)
    );

    let filtered = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api("/models?provider=aggregator&query=portable-model"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let filtered = response_json!(filtered, StatusCode::OK).await;
    assert_eq!(filtered["data"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["data"][0]["provider"], "aggregator");

    for (query, expected_provider) in [
        ("?provider=direct", "direct"),
        ("?provider=aggregator", "aggregator"),
        ("", "direct"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(api(&format!("/models/portable/test{query}")))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response_json!(response, StatusCode::OK).await;
        assert_eq!(body["model_id"], "portable-model");
        assert_eq!(body["provider"], expected_provider);
        assert_eq!(body["status"], "ok");
    }

    direct_probe.assert_calls(2);
    aggregator_probe.assert_calls(1);
}

#[tokio::test]
async fn test_model_invalid_mode_returns_400() {
    let state = test_app_state_with_env_lookup(
        default_test_server_settings(),
        RunLayer::default(),
        5,
        |_| None,
    );
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/models/claude-opus-4-6/test?mode=bogus"))
        .header("content-type", "application/json")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn test_model_invalid_reasoning_effort_returns_400() {
    let state = test_app_state_with_env_lookup(
        default_test_server_settings(),
        RunLayer::default(),
        5,
        |_| None,
    );
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/models/claude-opus-4-6/test?reasoning_effort=bogus"))
        .header("content-type", "application/json")
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn test_model_forwards_and_validates_reasoning_effort() {
    let upstream = MockServer::start();
    let completion = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .json_body_includes(r#"{"model":"acme-reasoner","reasoning_effort":"low"}"#);
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "id": "chatcmpl-test",
                "model": "acme-reasoner",
                "choices": [{
                    "message": {"role": "assistant", "content": "OK"},
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 1,
                    "completion_tokens": 1,
                    "total_tokens": 2
                }
            }));
    });
    let overlay = format!(
        r#"
[providers.acme]
display_name = "Acme"
adapter = "openai-compatible"
codec = "openai-chat"
base_url = {base_url}
auth = {{ type = "bearer" }}
priority = 120
default_model = "acme-reasoner"

[providers.acme.metadata.agent]
profile = "openai"

[providers.acme.models.acme-reasoner]
display_name = "Acme Reasoner"
api_model = "acme-reasoner"
limits = {{ context_tokens = 128000, max_output_tokens = 8192 }}
capabilities = {{ text = true, tools = true, reasoning = true, reasoning_effort = {{ minimal = false, low = true, medium = false, high = true, xhigh = false, max = false }} }}
protocol_options = {{ reasoning_effort_levels = true }}
"#,
        base_url = toml::Value::String(upstream.base_url()),
    );
    let state = TestAppStateBuilder::new()
        .llm_overlay_toml(&overlay)
        .vault_entries([("ACME_API_KEY", "acme-test-key")])
        .build();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api(
            "/models/acme-reasoner/test?provider=acme&reasoning_effort=low",
        ))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["status"], "ok");

    let unsupported = Request::builder()
        .method("POST")
        .uri(api(
            "/models/acme-reasoner/test?provider=acme&reasoning_effort=medium",
        ))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(unsupported).await.unwrap();
    let body = response_json!(response, StatusCode::BAD_REQUEST).await;
    assert_eq!(
        body["errors"][0]["detail"],
        "model 'acme-reasoner' does not support reasoning_effort 'medium'; allowed values: low, high"
    );
    completion.assert_calls(1);
}

#[tokio::test]
async fn test_provider_credentials_uses_app_state_catalog() {
    let upstream = MockServer::start();
    let completion = upstream.mock(|when, then| {
        when.method(POST)
            .path("/v1/chat/completions")
            .header("authorization", "Bearer sk-test");
        then.status(200)
            .header("content-type", "application/json")
            .json_body(json!({
                "id": "chatcmpl_test",
                "object": "chat.completion",
                "created": 1_700_000_000,
                "model": "test-model",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "OK"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
            }));
    });
    let overlay = acme_overlay(&upstream.base_url());
    let state = TestAppStateBuilder::new()
        .runtime_settings(default_test_server_settings(), RunLayer::default())
        .max_concurrent_runs(5)
        .llm_overlay_toml(&overlay)
        .build();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/providers/acme/credentials/test"))
        .header("content-type", "application/json")
        .body(Body::from(json!({ "api_key": "sk-test" }).to_string()))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["ok"], true);
    completion.assert();
}

#[tokio::test]
async fn list_models_filters_by_provider() {
    let app = test_app_with();

    let req = Request::builder()
        .method("GET")
        .uri(api("/models?provider=anthropic"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let models = body["data"].as_array().unwrap();
    assert!(!models.is_empty());
    assert!(
        models
            .iter()
            .all(|model| model["provider"] == serde_json::Value::String("anthropic".into()))
    );
}

#[tokio::test]
async fn list_models_exposes_reasoning_effort_controls() {
    let app = test_app_with();

    let req = Request::builder()
        .method("GET")
        .uri(api("/models?provider=moonshot"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let models = body["data"].as_array().unwrap();
    let kimi_k3 = models
        .iter()
        .find(|model| model["id"] == "kimi-k3")
        .expect("Kimi K3 should be listed");
    let kimi_k2_5 = models
        .iter()
        .find(|model| model["id"] == "kimi-k2.5")
        .expect("Kimi K2.5 should be listed");

    assert_eq!(
        kimi_k3["controls"]["reasoning_effort"],
        json!(["low", "high", "max"])
    );
    assert_eq!(kimi_k2_5["controls"]["reasoning_effort"], json!([]));
}

#[tokio::test]
async fn list_models_marks_configured_true_when_provider_has_credential_material() {
    let state = test_app_state_with_env_lookup(
        default_test_server_settings(),
        RunLayer::default(),
        5,
        |_| None,
    );
    state
        .stores
        .vault
        .set(
            EnvVars::ANTHROPIC_API_KEY,
            "test-key",
            SecretType::Token,
            None,
        )
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("GET")
        .uri(api("/models"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let models = body["data"].as_array().unwrap();

    assert!(models.iter().any(|model| model["provider"] != "anthropic"));
    assert!(models.iter().any(|model| model["provider"] == "anthropic"));
    assert!(
        models
            .iter()
            .filter(|model| model["provider"] == "anthropic")
            .all(|model| model["configured"].as_bool() == Some(true))
    );
    assert!(
        models
            .iter()
            .filter(|model| model["provider"] != "anthropic")
            .all(|model| model["configured"].as_bool() == Some(false))
    );
}

#[tokio::test]
async fn list_models_marks_configured_false_when_provider_cannot_register() {
    let overlay = acme_overlay("https://api.acme.test/v1");
    let state = TestAppStateBuilder::new()
        .runtime_settings(default_test_server_settings(), RunLayer::default())
        .max_concurrent_runs(5)
        .env_lookup(|name| (name == "ACME_API_KEY").then(|| "acme-key".to_string()))
        .llm_overlay_toml(&overlay)
        .build();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("GET")
        .uri(api("/models?provider=acme"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let models = body["data"].as_array().unwrap();

    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["id"], "acme-large");
    assert_eq!(models[0]["configured"].as_bool(), Some(false));
}

#[tokio::test]
async fn list_models_marks_configured_false_when_no_credential_material() {
    let state = test_app_state_with_env_lookup(
        default_test_server_settings(),
        RunLayer::default(),
        5,
        |_| None,
    );
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("GET")
        .uri(api("/models"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let models = body["data"].as_array().unwrap();

    assert!(!models.is_empty());
    assert!(
        models
            .iter()
            .all(|model| model["configured"].as_bool() == Some(false))
    );
}

#[tokio::test]
async fn list_models_unknown_provider_returns_empty_page() {
    let app = test_app_with();

    let req = Request::builder()
        .method("GET")
        .uri(api("/models?provider=missing-provider"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["data"].as_array().unwrap().len(), 0);
    assert_eq!(body["meta"]["has_more"].as_bool(), Some(false));
}

#[tokio::test]
async fn list_models_uses_app_state_catalog_overrides() {
    let overlay = acme_overlay("https://api.acme.test/v1");
    let state = TestAppStateBuilder::new()
        .llm_overlay_toml(&overlay)
        .build();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("GET")
        .uri(api("/models?provider=acme"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let models = body["data"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["id"], "acme-large");
    assert_eq!(models[0]["provider"], "acme");
}

#[tokio::test]
async fn list_providers_marks_configured_per_provider_and_omits_secrets() {
    // Only `ANTHROPIC_API_KEY` is supplied in the vault, so anthropic resolves as
    // configured while every other catalog provider does not.
    let state = test_app_state_with_env_lookup(
        default_test_server_settings(),
        RunLayer::default(),
        5,
        |_| None,
    );
    state
        .stores
        .vault
        .set(
            EnvVars::ANTHROPIC_API_KEY,
            "test-key",
            SecretType::Token,
            None,
        )
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("GET")
        .uri(api("/providers"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let providers = body["data"].as_array().unwrap();

    assert!(
        providers.len() >= 2,
        "builtin catalog should expose multiple providers"
    );

    let anthropic = providers
        .iter()
        .find(|provider| provider["id"] == "anthropic")
        .expect("anthropic provider should be present");
    assert_eq!(anthropic["configured"].as_bool(), Some(true));

    // `model_count` and `default_model` must reflect the catalog truth for
    // this exact provider, not merely be populated.
    let catalog = state_test_catalog();
    let anthropic_provider = catalog
        .enabled_provider("anthropic")
        .expect("anthropic should be listed");
    let expected_model_count = anthropic_provider.offerings().len();
    assert_eq!(
        anthropic["model_count"].as_u64(),
        Some(expected_model_count as u64),
        "anthropic model_count should match the catalog"
    );
    let expected_default = anthropic_provider
        .default_offering()
        .expect("anthropic should have a catalog default model");
    assert_eq!(
        anthropic["default_model"].as_str(),
        Some(expected_default.model.id().as_str()),
        "anthropic default_model should match the catalog"
    );

    assert!(
        providers
            .iter()
            .filter(|provider| provider["id"] != "anthropic")
            .all(|provider| provider["configured"].as_bool() == Some(false)),
        "providers without supplied credentials should be unconfigured"
    );

    // Internal-only catalog fields and the injected credential value must
    // never reach the wire.
    let serialized = body["data"].to_string();
    assert!(!serialized.contains("\"auth\""), "leaked `auth`");
    assert!(
        !serialized.contains("\"extra_headers\""),
        "leaked `extra_headers`"
    );
    assert!(
        !serialized.contains("\"billing_policy\""),
        "leaked `billing_policy`"
    );
    assert!(
        !serialized.contains("\"agent_profile\""),
        "leaked `agent_profile`"
    );
    assert!(
        !serialized.contains("test-key"),
        "leaked the injected credential value"
    );
}

#[tokio::test]
async fn list_providers_marks_all_unconfigured_without_credentials() {
    let state = test_app_state_with_env_lookup(
        default_test_server_settings(),
        RunLayer::default(),
        5,
        |_| None,
    );
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("GET")
        .uri(api("/providers"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let providers = body["data"].as_array().unwrap();

    assert!(!providers.is_empty());
    assert!(
        providers
            .iter()
            .all(|provider| provider["configured"].as_bool() == Some(false)),
        "no provider should be configured when no credentials are supplied"
    );
}

#[tokio::test]
async fn test_providers_no_configured_providers_returns_error_summary() {
    let state = test_app_state_with_env_lookup(
        default_test_server_settings(),
        RunLayer::default(),
        5,
        |_| None,
    );
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/providers/test"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;

    assert_eq!(body["data"].as_array().unwrap().len(), 0);
    assert_eq!(body["summary"]["status"], "error");
    assert_eq!(body["summary"]["total"], 0);
    assert_eq!(body["summary"]["passed"], 0);
    assert_eq!(body["summary"]["failed"], 0);
}

#[tokio::test]
async fn test_providers_successful_probe_returns_probe_model() {
    let server = MockServer::start_async().await;
    let response_mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/responses")
                .header("authorization", "Bearer vault-openai-key");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(openai_responses_payload("OK"));
        })
        .await;
    let state = TestAppStateBuilder::new()
        .runtime_settings(default_test_server_settings(), RunLayer::default())
        .max_concurrent_runs(5)
        .provider_base_url("openai", server.url("/v1"))
        .build();
    state
        .stores
        .vault
        .set(
            EnvVars::OPENAI_API_KEY,
            "vault-openai-key",
            SecretType::Token,
            None,
        )
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/providers/test"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let results = body["data"].as_array().unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["provider"], "openai");
    assert_eq!(results[0]["model_id"], "gpt-5.4-mini");
    assert_eq!(results[0]["status"], "ok");
    assert!(results[0]["error_message"].is_null());
    assert_eq!(body["summary"]["status"], "ok");
    assert_eq!(body["summary"]["total"], 1);
    assert_eq!(body["summary"]["passed"], 1);
    assert_eq!(body["summary"]["failed"], 0);
    response_mock.assert_async().await;
}

#[tokio::test]
async fn test_providers_auth_issue_returns_error_without_upstream_call() {
    let server = MockServer::start_async().await;
    let upstream = server
        .mock_async(|when, then| {
            when.method(POST).path("/v1/responses");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(openai_responses_payload("unexpected"));
        })
        .await;
    let state = TestAppStateBuilder::new()
        .runtime_settings(default_test_server_settings(), RunLayer::default())
        .max_concurrent_runs(5)
        .provider_base_url("openai", server.url("/v1"))
        .build();
    let mut credential = openai_oauth_credential();
    credential.tokens.expires_at = Utc::now() - ChronoDuration::hours(1);
    credential.tokens.refresh_token = None;
    state
        .stores
        .vault
        .set(
            "OPENAI_CODEX",
            &serde_json::to_string(&credential).unwrap(),
            SecretType::Oauth,
            None,
        )
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/providers/test"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let results = body["data"].as_array().unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["provider"], "openai-codex");
    assert!(results[0]["model_id"].is_null());
    assert_eq!(results[0]["status"], "error");
    assert!(
        results[0]["error_message"]
            .as_str()
            .unwrap()
            .contains("requires re-authentication")
    );
    assert_eq!(body["summary"]["status"], "error");
    assert_eq!(body["summary"]["total"], 1);
    assert_eq!(body["summary"]["passed"], 0);
    assert_eq!(body["summary"]["failed"], 1);
    upstream.assert_calls_async(0).await;
}

#[tokio::test]
async fn test_providers_registration_issue_returns_error_without_probe() {
    // An adapter lithos does not ship cannot be built, so the provider is
    // configured (it has a vault key) yet unavailable.
    let overlay = acme_overlay("https://api.acme.test/v1").replace(
        "adapter = \"openai-compatible\"",
        "adapter = \"not-an-adapter\"",
    );
    let state = TestAppStateBuilder::new()
        .runtime_settings(default_test_server_settings(), RunLayer::default())
        .max_concurrent_runs(5)
        .llm_overlay_toml(&overlay)
        .build();
    state
        .stores
        .vault
        .set("ACME_API_KEY", "acme-key", SecretType::Token, None)
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/providers/test"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let results = body["data"].as_array().unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["provider"], "acme");
    assert!(results[0]["model_id"].is_null());
    assert_eq!(results[0]["status"], "error");
    assert!(
        results[0]["error_message"]
            .as_str()
            .unwrap()
            .contains("not-an-adapter")
    );
    assert_eq!(body["summary"]["status"], "error");
    assert_eq!(body["summary"]["total"], 1);
    assert_eq!(body["summary"]["passed"], 0);
    assert_eq!(body["summary"]["failed"], 1);
}

#[tokio::test]
async fn test_providers_mixed_results_preserve_catalog_order_and_counts() {
    let server = MockServer::start_async().await;
    let alpha_mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/responses")
                .header("authorization", "Bearer alpha-key");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(openai_responses_payload("OK"));
        })
        .await;
    let zeta_mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/responses")
                .header("authorization", "Bearer zeta-key");
            then.status(401)
                .header("content-type", "application/json")
                .json_body(json!({
                    "error": {
                        "message": "invalid api key",
                        "type": "invalid_request_error"
                    }
                }));
        })
        .await;
    let overlay = format!(
        r#"
[providers.zeta]
display_name = "Zeta"
adapter = "openai"
codec = "openai-responses"
base_url = {base_url}
auth = {{ type = "bearer" }}
priority = 50
default_model = "zeta-probe"

[providers.zeta.models.zeta-probe]
display_name = "Zeta Probe"
api_model = "zeta-probe"
limits = {{ context_tokens = 128000, max_output_tokens = 8192 }}
capabilities = {{ text = true, tools = true }}
probe = true

[providers.alpha]
display_name = "Alpha"
adapter = "openai"
codec = "openai-responses"
base_url = {base_url}
auth = {{ type = "bearer" }}
priority = 40
default_model = "alpha-probe"

[providers.alpha.models.alpha-probe]
display_name = "Alpha Probe"
api_model = "alpha-probe"
limits = {{ context_tokens = 128000, max_output_tokens = 8192 }}
capabilities = {{ text = true, tools = true }}
probe = true

"#,
        base_url = toml::Value::String(server.base_url()),
    );
    let state = TestAppStateBuilder::new()
        .runtime_settings(default_test_server_settings(), RunLayer::default())
        .max_concurrent_runs(5)
        .llm_overlay_toml(&overlay)
        .build();
    state
        .stores
        .vault
        .set("ALPHA_API_KEY", "alpha-key", SecretType::Token, None)
        .await
        .unwrap();
    state
        .stores
        .vault
        .set("ZETA_API_KEY", "zeta-key", SecretType::Token, None)
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/providers/test"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let results = body["data"].as_array().unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["provider"], "alpha");
    assert_eq!(results[0]["model_id"], "alpha-probe");
    assert_eq!(results[0]["status"], "ok");
    assert_eq!(results[1]["provider"], "zeta");
    assert_eq!(results[1]["model_id"], "zeta-probe");
    assert_eq!(results[1]["status"], "error");
    assert_eq!(body["summary"]["status"], "error");
    assert_eq!(body["summary"]["total"], 2);
    assert_eq!(body["summary"]["passed"], 1);
    assert_eq!(body["summary"]["failed"], 1);
    alpha_mock.assert_async().await;
    zeta_mock.assert_async().await;
}

#[tokio::test]
async fn test_providers_response_does_not_leak_api_keys() {
    let leaked_key = "sk-proj-abcdefghijklmnopqrstuvwxyz0123456789";
    let server = MockServer::start_async().await;
    let response_mock = server
        .mock_async(move |when, then| {
            when.method(POST)
                .path("/v1/responses")
                .header("authorization", format!("Bearer {leaked_key}"));
            then.status(401)
                .header("content-type", "application/json")
                .json_body(json!({
                    "error": {
                        "message": format!("invalid api key {leaked_key}"),
                        "type": "invalid_request_error"
                    }
                }));
        })
        .await;
    let state = TestAppStateBuilder::new()
        .runtime_settings(default_test_server_settings(), RunLayer::default())
        .max_concurrent_runs(5)
        .provider_base_url("openai", server.url("/v1"))
        .build();
    state
        .stores
        .vault
        .set(EnvVars::OPENAI_API_KEY, leaked_key, SecretType::Token, None)
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/providers/test"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let serialized = body.to_string();

    assert!(
        !serialized.contains(leaked_key),
        "provider test response leaked API key: {serialized}"
    );
    assert!(
        serialized.contains("REDACTED"),
        "provider test response should include a redacted error: {serialized}"
    );
    response_mock.assert_async().await;
}

#[tokio::test]
async fn test_providers_requires_user_auth() {
    let app = build_router(test_app_state(), test_auth_mode());

    let req = Request::builder()
        .method("POST")
        .uri(api("/providers/test"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn delete_run_retry_after_missing_provider_resource_removes_metadata() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    let graph = Graph::new("test");

    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunCreated {
            run_id,
            title: None,
            settings: serde_json::to_value(fabro_types::WorkflowSettings::default()).unwrap(),
            graph: serde_json::to_value(graph).unwrap(),
            workflow_source: None,
            labels: std::collections::BTreeMap::default(),
            source_directory: Some("/tmp/fabro-run".to_string()),
            workflow_slug: Some("test".to_string()),
            workflow_version_id: None,
            target: None,
            automation: None,
            provenance: test_support::test_run_provenance(),
            spec_blob: None,
            git: None,
            fork_source_ref: None,
            retried_from: None,
            parent_id: None,
            web_url: None,
        },
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
        workflow_event::Event::SandboxInitialized {
            provider:          SandboxProviderKind::DOCKER,
            id:                "missing-sandbox".to_string(),
            working_directory: "/tmp/fabro-missing-sandbox".to_string(),
            image:             None,
            snapshot:          None,
            repo_cloned:       Some(false),
            clone_origin_url:  None,
            clone_branch:      None,
            workspace_root:    None,
            repos_root:        None,
            primary_repo_path: None,
            primary_repo_link: None,
        },
        workflow_event::Event::WorkflowRunCompleted {
            timing:               fabro_types::RunTiming::wall_only(1),
            artifact_count:       0,
            status:               "succeeded".to_string(),
            reason:               SuccessReason::Completed,
            failure:              None,
            final_git_commit_sha: None,
            final_patch:          None,
            diff_summary:         None,
            usage:                None,
        },
    ])
    .await;

    let req = Request::builder()
        .method("DELETE")
        .uri(api(&format!("/runs/{run_id}")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    response_json!(response, StatusCode::CONFLICT).await;

    let req = Request::builder()
        .method("DELETE")
        .uri(api(&format!("/runs/{run_id}")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NO_CONTENT).await;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}")))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}
