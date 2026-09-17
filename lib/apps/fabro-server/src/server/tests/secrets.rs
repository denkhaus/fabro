use super::*;

#[tokio::test]
async fn create_secret_stores_file_secret_outside_token_lookups() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let req = Request::builder()
        .method("POST")
        .uri(api("/secrets"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "/tmp/test.pem",
                "value": "pem-data",
                "type": "file",
                "description": "Test certificate",
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["name"], "/tmp/test.pem");
    assert_eq!(body["type"], "file");
    assert_eq!(body["description"], "Test certificate");

    let vault = state.stores.vault.snapshot().await.unwrap();
    assert_eq!(
        vault.get_entry("/tmp/test.pem").unwrap().secret_type,
        SecretType::File
    );
    assert_eq!(vault.file_secrets(), vec![(
        "/tmp/test.pem".to_string(),
        "pem-data".to_string()
    )]);
}

#[tokio::test]
async fn create_secret_rejects_bootstrap_secret_names() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    for name in [EnvVars::SESSION_SECRET, EnvVars::FABRO_DEV_TOKEN] {
        let response = app
            .clone()
            .oneshot(create_token_secret_request(name, "secret-value"))
            .await
            .unwrap();
        let body = response_json!(response, StatusCode::BAD_REQUEST).await;

        assert_eq!(
            body["errors"][0]["detail"],
            format!("{name} is a bootstrap secret; configure it with process env or server.env")
        );
        assert!(state.stores.vault.get(name).await.unwrap().is_none());
    }
}

#[tokio::test]
async fn create_secret_allows_optional_vault_and_custom_secret_names() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    for (name, value) in [
        (EnvVars::GITHUB_APP_CLIENT_SECRET, "github-client-secret"),
        ("CUSTOM_WORKFLOW_TOKEN", "custom-secret"),
    ] {
        let response = app
            .clone()
            .oneshot(create_token_secret_request(name, value))
            .await
            .unwrap();

        assert_status!(response, StatusCode::OK).await;
        assert_eq!(
            state
                .stores
                .vault
                .get(name)
                .await
                .unwrap()
                .map(|entry| entry.value),
            Some(value.to_string())
        );
    }
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
async fn create_secret_stores_valid_oauth_entries() {
    let state = TestAppStateBuilder::new().build();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let req = Request::builder()
        .method("POST")
        .uri(api("/secrets"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "OPENAI_CODEX",
                "value": openai_oauth_credential_json(),
                "type": "oauth"
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::OK).await;
    let listed = state.stores.vault.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "OPENAI_CODEX");
    assert_eq!(listed[0].secret_type, SecretType::Oauth);
    assert!(
        state
            .stores
            .vault
            .get("OPENAI_CODEX")
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn create_secret_rejects_under_scoped_daytona_api_key_and_leaves_vault_unchanged() {
    let server = MockServer::start_async().await;
    let auth = mock_daytona_auth_probe(&server).await;
    let current_key = mock_daytona_current_key(&server, vec![
        "delete:snapshots",
        "delete:sandboxes",
        "delete:volumes",
    ])
    .await;
    let base_url = server.base_url();
    let state = test_app_state_with_env_lookup(
        default_test_server_settings(),
        fabro_config::RunLayer::default(),
        5,
        move |name| match name {
            EnvVars::DAYTONA_API_URL => Some(base_url.clone()),
            _ => None,
        },
    );
    state
        .stores
        .vault
        .set(
            EnvVars::DAYTONA_API_KEY,
            "existing",
            SecretType::Token,
            None,
        )
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let req = Request::builder()
        .method("POST")
        .uri(api("/secrets"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": EnvVars::DAYTONA_API_KEY,
                "value": "dtn_test",
                "type": "token"
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::UNPROCESSABLE_ENTITY).await;

    assert_eq!(
        body["errors"][0]["detail"],
        "Daytona API key is missing required scopes: write:snapshots, write:sandboxes. \
         Regenerate the key with all snapshot and sandbox scopes."
    );
    assert_eq!(
        state
            .stores
            .vault
            .get(EnvVars::DAYTONA_API_KEY)
            .await
            .unwrap()
            .map(|entry| entry.value),
        Some("existing".to_string())
    );
    auth.assert_async().await;
    current_key.assert_async().await;
}

#[tokio::test]
async fn diagnostics_reports_under_scoped_daytona_api_key() {
    let server = MockServer::start_async().await;
    let auth = mock_daytona_auth_probe(&server).await;
    let current_key = mock_daytona_current_key(&server, vec![
        "delete:snapshots",
        "delete:sandboxes",
        "delete:volumes",
    ])
    .await;
    let base_url = server.base_url();
    let settings = fabro_config::ServerSettingsBuilder::from_toml(
        r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.sandbox.providers.docker]
enabled = false
"#,
    )
    .expect("settings should parse");
    let state = test_app_state_with_env_lookup(
        settings,
        fabro_config::RunLayer::default(),
        5,
        move |name| match name {
            EnvVars::DAYTONA_API_URL => Some(base_url.clone()),
            _ => None,
        },
    );
    state
        .stores
        .vault
        .set(
            EnvVars::DAYTONA_API_KEY,
            "dtn_test",
            SecretType::Token,
            None,
        )
        .await
        .unwrap();

    let report = crate::diagnostics::run_all(&state).await;
    let cloud_sandbox = report
        .sections
        .iter()
        .flat_map(|section| &section.checks)
        .find(|check| check.name == "Cloud Sandbox")
        .expect("cloud sandbox check should be present");

    assert_eq!(cloud_sandbox.status, CheckStatus::Error);
    assert_eq!(
        cloud_sandbox.summary,
        "Daytona API key is missing required scopes"
    );
    assert_eq!(
        cloud_sandbox.details[0].text,
        "missing: write:snapshots, write:sandboxes"
    );
    assert_eq!(
        cloud_sandbox.remediation.as_deref(),
        Some(
            "Regenerate the Daytona API key with scopes: write:snapshots, \
             delete:snapshots, write:sandboxes, delete:sandboxes, then \
             `fabro secret set DAYTONA_API_KEY`."
        )
    );
    auth.assert_async().await;
    current_key.assert_async().await;
}

#[tokio::test]
async fn resolve_llm_client_reads_openai_token_from_vault() {
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

    let llm_result = state.resolve_llm_client().await.unwrap();

    assert_eq!(llm_result.provider_ids(), vec![
        lithos_llm::catalog::builtin::openai()
    ]);
    assert!(llm_result.auth_issues.is_empty());
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
async fn resolve_llm_client_uses_vault_key_without_env_lookup_openai_settings() {
    let server = MockServer::start_async().await;
    let response_mock = server
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/responses")
                .header("authorization", "Bearer vault-openai-key");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(openai_responses_payload("hello from vault key"));
        })
        .await;
    let state = TestAppStateBuilder::new()
        .runtime_settings(default_test_server_settings(), RunLayer::default())
        .max_concurrent_runs(5)
        .env_lookup(|name| match name {
            "OPENAI_ORG_ID" => Some("env-org".to_string()),
            _ => None,
        })
        .provider_base_url("openai", server.url("/v1"))
        .build();
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

    let llm_result = state.resolve_llm_client().await.unwrap();
    let response = llm_result
        .client
        .complete(
            LlmRequest::builder()
                .model("openai/gpt-5.4")
                .user("Hello")
                .build()
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.text(), "hello from vault key");
    response_mock.assert_async().await;
}

#[tokio::test]
async fn list_secrets_includes_oauth_metadata() {
    let state = test_app_state();
    state
        .stores
        .vault
        .set(
            "OPENAI_CODEX",
            &openai_oauth_credential_json(),
            SecretType::Oauth,
            Some("saved auth"),
        )
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api("/secrets"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response_json!(response, StatusCode::OK).await;
    let data = body["data"].as_array().expect("data should be an array");
    let entry = data
        .iter()
        .find(|entry| entry["name"] == "OPENAI_CODEX")
        .expect("oauth metadata should be listed");
    assert_eq!(entry["type"], "oauth");
    assert_eq!(entry["description"], "saved auth");
    assert!(entry.get("updated_at").is_some());
    assert!(entry.get("value").is_none());
}

#[tokio::test]
async fn create_secret_rejects_invalid_oauth_json() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/secrets"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "OPENAI_CODEX",
                "value": "{not-json",
                "type": "oauth"
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn create_secret_rejects_invalid_oauth_name() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/secrets"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "1OPENAI",
                "value": openai_oauth_credential_json(),
                "type": "oauth"
            }))
            .unwrap(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::BAD_REQUEST).await;
}

#[test]
fn server_secrets_resolve_bootstrap_process_env_before_server_env() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("server.env"),
        "SESSION_SECRET=file-value\nFABRO_DEV_TOKEN=file-dev-token\n",
    )
    .unwrap();

    let secrets = ServerSecrets::load(
        dir.path().join("server.env"),
        HashMap::from([("SESSION_SECRET".to_string(), "env-value".to_string())]),
    )
    .unwrap();

    assert_eq!(secrets.get("SESSION_SECRET").as_deref(), Some("env-value"));
    assert_eq!(
        secrets.get("FABRO_DEV_TOKEN").as_deref(),
        Some("file-dev-token")
    );
}

#[test]
fn slack_service_ignores_vault_tokens_when_config_is_absent() {
    let state = slack_app_state_with_secret_sources(&slack_test_vault_tokens(), HashMap::new());

    assert!(state.slack_service.is_none());
}

#[test]
fn slack_service_is_enabled_by_config_and_vault_tokens() {
    let state = slack_app_state_with_settings_and_secret_sources(
        server_settings_from_toml(
            r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.integrations.slack]
enabled = true
"#,
        ),
        &[
            (
                EnvVars::FABRO_SLACK_BOT_TOKEN,
                "xoxb-test",
                SecretType::Token,
            ),
            (
                EnvVars::FABRO_SLACK_APP_TOKEN,
                "xapp-test",
                SecretType::Token,
            ),
        ],
        HashMap::new(),
    );

    let service = state
        .slack_service
        .as_ref()
        .expect("slack service should be enabled by config and vault tokens");
    let connection = service.connection_status();
    assert_eq!(connection.kind, IntegrationConnectionKind::SocketMode);
    assert_eq!(connection.status, IntegrationConnectionState::Connecting);
    assert!(connection.last_connected_at.is_none());
    assert!(connection.last_error.is_none());
    assert!(service.default_channel.is_none());
}

#[test]
fn slack_service_receives_configured_default_channel_verbatim() {
    let state = slack_app_state_with_settings_and_secret_sources(
        server_settings_from_toml(
            r##"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.integrations.slack]
enabled = true
default_channel = "#releases"
"##,
        ),
        &slack_test_vault_tokens(),
        HashMap::new(),
    );

    let service = state
        .slack_service
        .as_ref()
        .expect("slack service should be enabled by config and vault tokens");
    assert_eq!(service.default_channel.as_deref(), Some("#releases"));
}

#[test]
fn slack_service_ignores_server_env_tokens() {
    let state = slack_app_state_with_secret_sources(
        &[],
        HashMap::from([
            (
                EnvVars::FABRO_SLACK_BOT_TOKEN.to_string(),
                "xoxb-server-env".to_string(),
            ),
            (
                EnvVars::FABRO_SLACK_APP_TOKEN.to_string(),
                "xapp-server-env".to_string(),
            ),
        ]),
    );

    assert!(state.slack_service.is_none());
}

#[test]
fn slack_service_respects_disabled_server_config_even_with_vault_tokens() {
    let mut settings = default_test_server_settings();
    settings.server.integrations.slack.enabled = false;
    let (store, artifact_store) = test_store_bundle();
    let vault_path = test_secret_store_path();
    let mut vault = Vault::load(vault_path.clone()).unwrap();
    vault
        .set(
            EnvVars::FABRO_SLACK_BOT_TOKEN,
            "xoxb-test",
            SecretType::Token,
            None,
        )
        .unwrap();
    vault
        .set(
            EnvVars::FABRO_SLACK_APP_TOKEN,
            "xapp-test",
            SecretType::Token,
            None,
        )
        .unwrap();

    let state = build_app_state(AppStateConfig {
        resolved_settings: resolved_runtime_settings_for_tests(
            settings,
            RunLayer::default(),
            LlmLayer::default(),
        ),
        registry_factory_override: None,
        max_concurrent_runs: 5,
        store,
        artifact_store,
        db_pool: test_db_pool_for_vault_path(&vault_path).expect("test db pool should build"),
        preloaded_vault: vault,
        server_secrets: load_test_server_secrets(
            tempfile::tempdir().unwrap().path().join("server.env"),
            HashMap::new(),
        ),
        env_lookup: default_env_lookup(),
        github_api_base_url: None,
        active_config_path: tempfile::tempdir().unwrap().path().join("settings.toml"),
        http_client: Some(fabro_http::test_http_client().expect("test HTTP client should build")),
        sandbox_inventory: None,
        shutdown: tokio_util::sync::CancellationToken::new(),
        worker_control_bus: None,
        worker_runtime: None,
        automation_materializer_override: None,
        automation_breaker_notifier_override: None,
    })
    .expect("slack disabled test app state should build");

    assert!(state.slack_service.is_none());
}

#[test]
fn build_app_state_requires_session_secret_for_worker_tokens() {
    let server_settings = server_settings_from_toml(
        r#"
_version = 1

[server.auth]
methods = ["dev-token"]
"#,
    );
    let (store, artifact_store) = test_store_bundle();
    let vault_path = test_secret_store_path();
    let server_env_path = vault_path.with_file_name("server.env");
    let db_pool = test_db_pool_for_vault_path(&vault_path).expect("test db pool should build");
    let preloaded_vault = crate::test_support::test_secret_snapshot(db_pool.clone())
        .expect("test secret snapshot should build");
    let Err(err) = build_app_state(AppStateConfig {
        resolved_settings: resolved_runtime_settings_for_tests(
            server_settings,
            RunLayer::default(),
            LlmLayer::default(),
        ),
        registry_factory_override: None,
        max_concurrent_runs: 5,
        store,
        artifact_store,
        db_pool,
        preloaded_vault,
        server_secrets: ServerSecrets::load(server_env_path, HashMap::new()).unwrap(),
        env_lookup: default_env_lookup(),
        github_api_base_url: None,
        active_config_path: tempfile::tempdir().unwrap().path().join("settings.toml"),
        http_client: Some(fabro_http::test_http_client().expect("test HTTP client should build")),
        sandbox_inventory: None,
        shutdown: tokio_util::sync::CancellationToken::new(),
        worker_control_bus: None,
        worker_runtime: None,
        automation_materializer_override: None,
        automation_breaker_notifier_override: None,
    }) else {
        panic!("build_app_state should require SESSION_SECRET")
    };

    assert!(err.to_string().contains(
        "Fabro server refuses to start: auth is configured but SESSION_SECRET is not set."
    ));
}

#[tokio::test]
async fn github_token_strategy_reads_github_token_from_vault() {
    let state = create_github_token_app_state(Some("ghu_test"), None);
    let settings = state.server_settings();

    let credentials = state
        .github_credentials(&settings.server.integrations.github)
        .await
        .expect("vault GitHub token should resolve")
        .expect("vault GitHub token should produce credentials");

    assert!(
        matches!(credentials, fabro_github::GitHubCredentials::Pat(token) if token == "ghu_test")
    );
}

/// What a node handler observed about the GitHub credential bridge while the
/// workflow executed.

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
