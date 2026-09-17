use super::*;

#[tokio::test]
async fn router_redirects_web_page_requests_to_canonical_host() {
    let app = canonical_host_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri("/login")
                .header(header::HOST, "localhost:32276")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let response = checked_response!(response, StatusCode::PERMANENT_REDIRECT).await;
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "http://127.0.0.1:32276/login"
    );
}

#[tokio::test]
async fn router_does_not_redirect_api_requests_to_canonical_host() {
    let app = canonical_host_test_app();

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(api("/openapi.json"))
                .header(header::HOST, "localhost:32276")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_status!(response, StatusCode::OK).await;
}

#[test]
fn replace_settings_rejects_invalid_canonical_origin_and_keeps_previous_settings() {
    for invalid in [
        "",
        "/relative/path",
        "ftp://fabro.example.com",
        "http://0.0.0.0:32276",
    ] {
        // No FABRO_WEB_URL override: web.url is plain config now, so the
        // invalid value is rejected from the settings literal and the kept
        // previous settings stay valid.
        let state = test_app_state_with_env_lookup(
            canonical_origin_settings("http://valid.example.com"),
            RunLayer::default(),
            5,
            |_| None,
        );

        let err = state
            .replace_runtime_settings(resolved_runtime_settings_from_toml(&format!(
                r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
url = "{invalid}"
"#,
            )))
            .expect_err("invalid canonical origin should be rejected");
        assert!(
            err.to_string()
                .contains("server.web.url is required and must be an absolute http(s) URL"),
            "unexpected error for {invalid}: {err}"
        );
        assert_eq!(
            state.canonical_origin().unwrap(),
            "http://valid.example.com".to_string()
        );
    }
}

#[test]
fn canonical_origin_prefers_fabro_web_url_env_override() {
    // FABRO_WEB_URL is the native control-plane override; it wins over the
    // plain `server.web.url` settings literal.
    let state = test_app_state_with_env_lookup(
        canonical_origin_settings("http://settings.example.com"),
        RunLayer::default(),
        5,
        |name| (name == "FABRO_WEB_URL").then(|| "http://env.example.com".to_string()),
    );

    assert_eq!(state.canonical_origin().unwrap(), "http://env.example.com");
}

#[test]
fn canonical_origin_uses_settings_literal_without_env_override() {
    // Without FABRO_WEB_URL set, the plain `server.web.url` literal is used.
    let state = test_app_state_with_env_lookup(
        canonical_origin_settings("http://settings.example.com"),
        RunLayer::default(),
        5,
        |_| None,
    );

    assert_eq!(
        state.canonical_origin().unwrap(),
        "http://settings.example.com"
    );
}

#[test]
fn replace_settings_updates_layer_and_typed_server_settings() {
    let state = test_app_state_with_options(
        server_settings_from_toml(
            r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
url = "http://old.example.com"

[server.storage]
root = "/srv/old"
"#,
        ),
        manifest_run_defaults_from_toml(
            r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
url = "http://old.example.com"

[server.storage]
root = "/srv/old"
"#,
        ),
        5,
    );

    let updated = r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
url = "http://new.example.com"

[run.execution]
mode = "dry_run"

[server.storage]
root = "/srv/new"
"#;

    state
        .replace_runtime_settings(resolved_runtime_settings_from_toml(updated))
        .expect("valid settings should replace current state");

    assert_eq!(state.canonical_origin().unwrap(), "http://new.example.com");
    assert_eq!(state.server_settings().server.storage.root, "/srv/new");
    assert_eq!(
        state
            .manifest_run_settings()
            .expect("manifest run settings should resolve")
            .execution
            .mode,
        RunMode::DryRun
    );
    let manifest_run_defaults = state.manifest_run_defaults();
    assert_eq!(
        manifest_run_defaults
            .execution
            .as_ref()
            .and_then(|execution| execution.mode),
        Some(RunMode::DryRun)
    );
}

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

#[cfg(unix)]
#[test]
fn worker_command_uses_null_stdin_and_token_env() {
    let storage_dir = tempfile::tempdir().unwrap();
    let state = worker_command_test_state(storage_dir.path(), &["dev-token"], Some(TEST_DEV_TOKEN));
    let cmd = worker_command(
        state.as_ref(),
        RunId::new(),
        RunExecutionMode::Start,
        storage_dir.path(),
        false,
    )
    .unwrap();

    assert_worker_command_passes_token_only_by_env(&cmd);
}

#[cfg(unix)]
#[test]
fn worker_command_sets_worker_args() {
    let storage_dir = tempfile::tempdir().unwrap();
    let run_dir = storage_dir.path().join("run-scratch");
    let state = worker_command_test_state(storage_dir.path(), &["dev-token"], Some(TEST_DEV_TOKEN));
    let run_id = RunId::new();

    let cmd = worker_command(
        state.as_ref(),
        run_id,
        RunExecutionMode::Resume,
        &run_dir,
        false,
    )
    .unwrap();

    let args = cmd
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(args, vec![
        "__run-worker".to_string(),
        "--server".to_string(),
        "http://127.0.0.1:32276".to_string(),
        "--storage-dir".to_string(),
        storage_dir.path().display().to_string(),
        "--run-dir".to_string(),
        run_dir.display().to_string(),
        "--run-id".to_string(),
        run_id.to_string(),
        "--mode".to_string(),
        "resume".to_string(),
    ]);
}

#[cfg(unix)]
#[test]
fn worker_command_default_token_omits_agent_run_tools_scope() {
    let storage_dir = tempfile::tempdir().unwrap();
    let state = worker_command_test_state(storage_dir.path(), &["dev-token"], Some(TEST_DEV_TOKEN));
    let run_id = RunId::new();

    let cmd = worker_command(
        state.as_ref(),
        run_id,
        RunExecutionMode::Start,
        storage_dir.path(),
        false,
    )
    .unwrap();

    assert_worker_command_passes_token_only_by_env(&cmd);
    let claims = worker_token_claims(&cmd, state.as_ref());

    assert_eq!(claims.run_id, run_id.to_string());
    assert_eq!(claims.scope.split_whitespace().collect::<Vec<_>>(), vec![
        "run:worker"
    ]);
}

#[cfg(unix)]
#[test]
fn worker_command_opt_in_token_includes_agent_run_tools_scope() {
    let storage_dir = tempfile::tempdir().unwrap();
    let state = worker_command_test_state(storage_dir.path(), &["dev-token"], Some(TEST_DEV_TOKEN));
    let run_id = RunId::new();

    let cmd = worker_command(
        state.as_ref(),
        run_id,
        RunExecutionMode::Start,
        storage_dir.path(),
        true,
    )
    .unwrap();

    assert_worker_command_passes_token_only_by_env(&cmd);
    let claims = worker_token_claims(&cmd, state.as_ref());

    assert_eq!(claims.run_id, run_id.to_string());
    assert_eq!(claims.scope.split_whitespace().collect::<Vec<_>>(), vec![
        "run:worker",
        "agent:run_tools"
    ]);
}

#[cfg(unix)]
#[test]
fn worker_command_forwards_github_app_private_key_from_vault() {
    let storage_dir = tempfile::tempdir().unwrap();
    let state = worker_command_test_state(storage_dir.path(), &["dev-token"], Some(TEST_DEV_TOKEN));
    let spec = worker_launch_spec(
        state.as_ref(),
        RunId::new(),
        RunExecutionMode::Start,
        storage_dir.path(),
        false,
        &[],
        Some("test-private-key".to_string()),
    )
    .unwrap();
    let cmd = LocalWorkerRuntime::command_for_spec(&spec);

    assert_eq!(
        command_env_value(&cmd, EnvVars::GITHUB_APP_PRIVATE_KEY),
        EnvOverride::Set("test-private-key".to_string())
    );
}

#[cfg(unix)]
#[test]
fn worker_command_omits_github_app_private_key_when_unset() {
    let storage_dir = tempfile::tempdir().unwrap();
    let state = worker_command_test_state(storage_dir.path(), &["dev-token"], Some(TEST_DEV_TOKEN));
    let cmd = worker_command(
        state.as_ref(),
        RunId::new(),
        RunExecutionMode::Start,
        storage_dir.path(),
        false,
    )
    .unwrap();

    assert_eq!(
        command_env_value(&cmd, EnvVars::GITHUB_APP_PRIVATE_KEY),
        EnvOverride::Unchanged
    );
}

#[cfg(unix)]
#[test]
fn worker_command_sets_fabro_log_from_server_logging_config() {
    let storage_dir = tempfile::tempdir().unwrap();
    let state = worker_command_test_state_with_extra_config(
        storage_dir.path(),
        &["dev-token"],
        Some(TEST_DEV_TOKEN),
        r#"
[server.logging]
level = "debug"
"#,
    );
    let run_id = RunId::new();

    let cmd = worker_command(
        state.as_ref(),
        run_id,
        RunExecutionMode::Start,
        storage_dir.path(),
        false,
    )
    .unwrap();

    assert_eq!(
        command_env_value(&cmd, EnvVars::FABRO_LOG),
        EnvOverride::Set("debug".to_string())
    );
}

#[cfg(unix)]
#[test]
fn worker_command_sets_fabro_log_destination_from_server_logging_config() {
    let storage_dir = tempfile::tempdir().unwrap();
    let state = worker_command_test_state_with_extra_config(
        storage_dir.path(),
        &["dev-token"],
        Some(TEST_DEV_TOKEN),
        r#"
[server.logging]
destination = "stdout"
"#,
    );
    let run_id = RunId::new();

    let cmd = worker_command(
        state.as_ref(),
        run_id,
        RunExecutionMode::Start,
        storage_dir.path(),
        false,
    )
    .unwrap();

    assert_eq!(
        command_env_value(&cmd, EnvVars::FABRO_LOG_DESTINATION),
        EnvOverride::Set("stdout".to_string())
    );
}

#[cfg(unix)]
#[test]
fn worker_command_sets_fabro_config_to_active_absolute_config_path() {
    let storage_dir = tempfile::tempdir().unwrap();
    let config_dir = tempfile::tempdir().unwrap();
    let active_config_path = config_dir.path().join("settings.toml");
    let state = worker_command_test_state_with_active_config_path(
        storage_dir.path(),
        &["dev-token"],
        Some(TEST_DEV_TOKEN),
        active_config_path.clone(),
    );
    let run_id = RunId::new();

    let cmd = worker_command(
        state.as_ref(),
        run_id,
        RunExecutionMode::Start,
        storage_dir.path(),
        false,
    )
    .unwrap();

    assert!(active_config_path.is_absolute());
    assert_eq!(
        command_env_value(&cmd, EnvVars::FABRO_CONFIG),
        EnvOverride::Set(active_config_path.display().to_string())
    );
    let worker_args = cmd
        .as_std()
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(
        !worker_args.iter().any(|arg| arg == "--config"),
        "__run-worker argument contract should not grow hidden config args: {worker_args:?}"
    );
}

#[cfg(unix)]
#[test]
fn worker_command_env_log_destination_overrides_server_logging_config() {
    let storage_dir = tempfile::tempdir().unwrap();
    let state = worker_command_test_state_with_extra_config_and_env_lookup(
        storage_dir.path(),
        &["dev-token"],
        Some(TEST_DEV_TOKEN),
        r#"
[server.logging]
destination = "file"
"#,
        &[],
        |name| (name == EnvVars::FABRO_LOG_DESTINATION).then(|| "stdout".to_string()),
    );
    let run_id = RunId::new();

    let cmd = worker_command(
        state.as_ref(),
        run_id,
        RunExecutionMode::Start,
        storage_dir.path(),
        false,
    )
    .unwrap();

    assert_eq!(
        command_env_value(&cmd, EnvVars::FABRO_LOG_DESTINATION),
        EnvOverride::Set("stdout".to_string())
    );
}

#[cfg(unix)]
#[test]
fn worker_command_rejects_invalid_env_log_destination() {
    let storage_dir = tempfile::tempdir().unwrap();
    let state = worker_command_test_state_with_extra_config_and_env_lookup(
        storage_dir.path(),
        &["dev-token"],
        Some(TEST_DEV_TOKEN),
        r#"
[server.logging]
destination = "file"
"#,
        &[],
        |name| (name == EnvVars::FABRO_LOG_DESTINATION).then(|| "stdot".to_string()),
    );
    let run_id = RunId::new();

    let Err(err) = worker_command(
        state.as_ref(),
        run_id,
        RunExecutionMode::Start,
        storage_dir.path(),
        false,
    ) else {
        panic!("invalid env destination should fail");
    };

    let message = err.to_string();
    assert!(message.contains(EnvVars::FABRO_LOG_DESTINATION));
    assert!(message.contains("stdot"));
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
async fn static_favicon_is_served() {
    let app = test_app_with();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/images/favicon.svg")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let response = checked_response!(response, StatusCode::OK).await;
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("image/svg+xml")
    );
}
