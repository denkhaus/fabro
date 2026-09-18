use super::*;

#[tokio::test(flavor = "current_thread")]
async fn http_log_omits_unset_optional_auth_fields() {
    let (_guard, events) = capture_server_logs();
    let app = test_app_with();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let events = events.lock().expect("captured log events").clone();
    assert_eq!(events.len(), 1);
    let field_names = events[0]
        .fields
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert!(field_names.contains(&"principal_kind"));
    assert!(field_names.contains(&"auth_status"));
    assert!(!field_names.contains(&"auth_error_code"));
    assert!(!field_names.contains(&"user_auth_method"));
    assert!(!field_names.contains(&"idp_issuer"));
    assert!(!field_names.contains(&"run_id"));
}

#[tokio::test(flavor = "current_thread")]
async fn http_log_records_user_principal_fields() {
    let (_state, app) = jwt_auth_app();
    let bearer = issue_test_user_jwt();
    let (_guard, events) = capture_server_logs();

    let response = app
        .oneshot(bearer_request(Method::GET, "/runs", &bearer, Body::empty()))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let events = events.lock().expect("captured log events").clone();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_log_field(event, "principal_kind", "user");
    assert_log_field(event, "auth_status", "authenticated");
    assert_log_field(event, "user_auth_method", "github");
    assert_log_field(event, "idp_issuer", "https://github.com");
    assert_log_field(event, "idp_subject", "12345");
    assert_log_field(event, "login", "octocat");
    assert_log_field_absent(event, "auth_error_code");
}

#[tokio::test(flavor = "current_thread")]
async fn http_log_records_webhook_principal_fields() {
    let body = br#"{"repository":{"full_name":"owner/repo"},"action":"opened"}"#;
    let signature = compute_signature(TEST_WEBHOOK_SECRET.as_bytes(), body);
    let app = webhook_test_app(dev_token_auth_mode());
    let (_guard, events) = capture_server_logs();

    let response = app
        .oneshot(webhook_request(Some(&signature), None, body))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let events = events.lock().expect("captured log events").clone();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_log_field(event, "principal_kind", "webhook");
    assert_log_field(event, "auth_status", "authenticated");
    assert_log_field(event, "delivery_id", "delivery-1");
    assert_log_field_absent(event, "auth_error_code");
}

#[tokio::test]
async fn github_token_strategy_ignores_process_env_token() {
    let state = create_github_token_app_state_with_env_lookup(None, None, |name| match name {
        EnvVars::GITHUB_TOKEN => Some("ghu_from_env".to_string()),
        _ => None,
    });
    let settings = state.server_settings();

    let err = state
        .github_credentials(&settings.server.integrations.github)
        .await
        .expect_err("server runtime should ignore env-backed GitHub tokens");

    assert_eq!(
        err.to_string(),
        "GITHUB_TOKEN not configured -- run fabro install or run fabro secret set GITHUB_TOKEN"
    );
}

#[tokio::test]
async fn github_token_strategy_ignores_gh_token_alias() {
    let state = create_github_token_app_state_with_env_lookup(None, None, |name| match name {
        EnvVars::GH_TOKEN => Some("ghu_from_env_alias".to_string()),
        _ => None,
    });
    state
        .stores
        .vault
        .set(
            EnvVars::GH_TOKEN,
            "ghu_from_vault_alias",
            SecretType::Token,
            None,
        )
        .await
        .unwrap();
    let settings = state.server_settings();

    let err = state
        .github_credentials(&settings.server.integrations.github)
        .await
        .expect_err("server runtime should ignore GH_TOKEN in env and vault");

    assert_eq!(
        err.to_string(),
        "GITHUB_TOKEN not configured -- run fabro install or run fabro secret set GITHUB_TOKEN"
    );
}

/// ADR-0019.6 wire proof (fabro-e505): a Worker-principal run whose
/// (agent-authored) config declares GitHub permissions executes nodes with
/// no credential bridge and no GITHUB_TOKEN in the stage env, while a
/// User-principal run of the same config keeps the scoped grant.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_start_grants_github_token_bridge_only_to_user_principals() {
    let source = r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[run.environment]
id = "local"

# Explicit author: the run's git identity resolves without a GitHub
# lookup — #856 would otherwise GET /user with the fake vault token and
# fail the run before the bridge observation. This test scopes to the
# credential bridge, not identity resolution.
[run.git.author]
name = "Bridge Test"
email = "bridge@test.invalid"

[run.integrations.github.permissions]
contents = "read"
"#;
    let observations = StdArc::new(StdMutex::new(Vec::new()));
    let capturing = StdArc::clone(&observations);
    let state = test_app_state_with_runtime_settings_environment_and_registry_factory(
        server_settings_from_toml(source),
        manifest_run_defaults_from_toml(source),
        // These runs execute a real in-process workflow; the default
        // DOCKER-seeded environment would require a Docker daemon, which is
        // absent in daemon-free test environments. Seed LOCAL instead.
        Some(SandboxProviderKind::LOCAL),
        move |interviewer| {
            let mut registry = fabro_workflow::handler::default_registry(interviewer, || None);
            registry.register(
                "wait",
                Box::new(BridgeCapturingWaitHandler {
                    observations: StdArc::clone(&capturing),
                }),
            );
            registry
        },
    );
    state
        .stores
        .vault
        .set(
            EnvVars::GITHUB_TOKEN,
            "ghu_e505_wire_test",
            SecretType::Token,
            None,
        )
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    // User principal (injected dev-user bearer): the declared permissions
    // resolve into a credential bridge for node execution.
    let run_settings = r#"
[run.git.author]
name = "Bridge Test"
email = "bridge@test.invalid"

[run.integrations.github.permissions]
contents = "read"
"#;
    let user_run_id = bridge_intent_run(&app, None, run_settings)
        .await
        .parse::<RunId>()
        .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api(&format!("/runs/{user_run_id}/start")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    response_json!(response, StatusCode::OK).await;
    execute_run(Arc::clone(&state), user_run_id).await;
    assert_eq!(
        state
            .stores
            .runs
            .open_run_reader(&user_run_id)
            .await
            .unwrap()
            .state()
            .await
            .unwrap()
            .status,
        RunStatus::Succeeded {
            reason: SuccessReason::Completed,
        },
        "user-principal run should complete so its observation is trustworthy"
    );

    // Worker principal: a run created by the user run's run-tools worker
    // token gets Worker provenance; the same declared permissions must NOT
    // mint a bridge.
    let worker_token = issue_test_run_tools_worker_token(&user_run_id);
    let worker_run_id = bridge_intent_run(&app, Some(&worker_token), run_settings)
        .await
        .parse::<RunId>()
        .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api(&format!("/runs/{worker_run_id}/start")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    response_json!(response, StatusCode::OK).await;
    execute_run(Arc::clone(&state), worker_run_id).await;
    assert_eq!(
        state
            .stores
            .runs
            .open_run_reader(&worker_run_id)
            .await
            .unwrap()
            .state()
            .await
            .unwrap()
            .status,
        RunStatus::Succeeded {
            reason: SuccessReason::Completed,
        },
        "worker-principal run should complete so its observation is trustworthy"
    );

    assert_eq!(
        observations
            .lock()
            .expect("bridge observation lock poisoned")
            .clone(),
        vec![
            GithubBridgeObservation {
                token_source_present: true,
                env_has_github_token: true,
            },
            GithubBridgeObservation {
                token_source_present: false,
                env_has_github_token: false,
            },
        ],
        "user run keeps the credential bridge; worker run executes credential-free"
    );
}

#[tokio::test]
async fn auth_login_github_redirects_to_github() {
    let source = r#"
_version = 1

[server.auth]
methods = ["github"]

[server.web]
enabled = true
url = "http://localhost:3000"

[server.auth.github]
allowed_usernames = ["octocat"]

[server.integrations.github]
app_id = "123"
client_id = "Iv1.testclient"
slug = "fabro"
"#;
    let app = build_router(
        test_app_state_with_session_key(
            server_settings_from_toml(source),
            manifest_run_defaults_from_toml(source),
            Some("github-redirect-test-key-0123456789"),
        ),
        AuthMode::Enabled(ConfiguredAuth {
            methods:    vec![ServerAuthMethod::Github],
            dev_token:  None,
            jwt_key:    None,
            jwt_issuer: None,
        }),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/auth/login/github")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let response = checked_response!(response, StatusCode::SEE_OTHER).await;
    let location = response
        .headers()
        .get(axum::http::header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .unwrap();
    assert!(location.starts_with("https://github.com/login/oauth/authorize?"));
}

#[tokio::test]
async fn logout_redirects_to_login_page() {
    let app = test_app_with();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/logout")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let response = checked_response!(response, StatusCode::SEE_OTHER).await;
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/login")
    );
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

#[tokio::test]
async fn dev_token_web_login_authorizes_cookie_backed_api_requests() {
    const DEV_TOKEN: &str =
        "fabro_dev_abababababababababababababababababababababababababababababababab";

    let state = test_app_state_with_session_key(
        default_test_server_settings(),
        RunLayer::default(),
        Some("server-test-session-key-0123456789"),
    );
    let app = build_router(
        Arc::clone(&state),
        AuthMode::Enabled(ConfiguredAuth {
            methods:    vec![ServerAuthMethod::DevToken],
            dev_token:  Some(DEV_TOKEN.to_string()),
            jwt_key:    Some(
                auth::derive_jwt_key(b"server-test-session-key-0123456789")
                    .expect("test JWT key should derive"),
            ),
            jwt_issuer: Some("https://fabro.example".to_string()),
        }),
    );

    let login_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login/dev-token")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({ "token": DEV_TOKEN }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let login_response = checked_response!(login_response, StatusCode::OK).await;
    let session_cookie = login_response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .expect("session cookie should be set")
        .to_string();

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api("/runs"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, &session_cookie)
                .body(Body::from(
                    test_intent_with_bearer(
                        &app,
                        "workflow.fabro",
                        MINIMAL_DOT,
                        None,
                        Some(DEV_TOKEN),
                    )
                    .await
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let create_body = response_json!(create_response, StatusCode::CREATED).await;
    let run_id = create_body["id"].as_str().unwrap();

    let state_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/state")))
                .header(header::COOKIE, &session_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let state_body = response_json!(state_response, StatusCode::OK).await;
    assert_eq!(
        state_body["spec"]["provenance"]["subject"]["auth_method"],
        "dev_token"
    );
    assert_eq!(state_body["spec"]["provenance"]["subject"]["login"], "dev");
}

#[tokio::test]
async fn worker_token_accepts_run_scoped_routes_and_falls_back_to_user_jwt() {
    let (state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_worker_token(&run_id);
    let other_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let other_worker_token = issue_test_worker_token(&other_run_id);
    let blob_hash = state
        .stores
        .runs
        .open_run(&run_id)
        .await
        .unwrap()
        .write_blob(b"preloaded blob")
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{run_id}/state"),
            &worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    for path in [
        format!("/runs/{run_id}"),
        format!("/runs/{run_id}/questions"),
    ] {
        let response = app
            .clone()
            .oneshot(bearer_request(
                Method::GET,
                &path,
                &worker_token,
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_status!(response, StatusCode::OK).await;
    }

    let append_body = serde_json::to_vec(&serde_json::json!({
        "id": "evt-run-notice",
        "ts": "2026-04-23T12:00:00Z",
        "event": "run.notice",
        "run_id": run_id.to_string(),
        "properties": {
            "level": "info",
            "code": "worker",
            "message": "hello"
        }
    }))
    .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api(&format!("/runs/{run_id}/events")))
                .header(header::AUTHORIZATION, format!("Bearer {worker_token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(append_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{run_id}/events"),
            &worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::POST,
            &format!("/runs/{run_id}/blobs"),
            &worker_token,
            Body::from("worker blob"),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{run_id}/blobs/{blob_hash}"),
            &worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{run_id}/state"),
            &user_jwt,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{run_id}/state"),
            &other_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;
}

#[tokio::test]
async fn run_tool_worker_cross_run_routes_require_inspects_scope() {
    // fabro-4556: an `agent:run_tools` worker no longer manages arbitrary
    // runs. Allowed targets: the worker's own run, runs of workflows its
    // token declares in `inspects`, runs the worker created, and runs
    // descended from it. Everything else on run-management routes is 403.
    let (state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let parent_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let foreign_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let run_tool_worker_token = issue_test_run_tools_worker_token(&parent_run_id);

    // Unchanged surfaces: enumeration and resolve stay worker-accessible.
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            "/runs",
            &run_tool_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/resolve?selector={foreign_run_id}"),
            &run_tool_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    // Foreign run: every run-management route denies the worker now.
    for path in [
        format!("/runs/{foreign_run_id}"),
        format!("/runs/{foreign_run_id}/state"),
        format!("/runs/{foreign_run_id}/events"),
        format!("/runs/{foreign_run_id}/questions"),
    ] {
        let response = app
            .clone()
            .oneshot(bearer_request(
                Method::GET,
                &path,
                &run_tool_worker_token,
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_status!(response, StatusCode::FORBIDDEN).await;
    }

    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            &format!("/runs/{foreign_run_id}/start"),
            &run_tool_worker_token,
            &json!({ "resume": false }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::POST,
            &format!("/runs/{foreign_run_id}/cancel"),
            &run_tool_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    for (method, path) in [
        (Method::POST, format!("/runs/{foreign_run_id}/archive")),
        (Method::POST, format!("/runs/{foreign_run_id}/unarchive")),
        (Method::POST, format!("/runs/{foreign_run_id}/interrupt")),
    ] {
        let response = app
            .clone()
            .oneshot(bearer_request(
                method,
                &path,
                &run_tool_worker_token,
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_status!(response, StatusCode::FORBIDDEN).await;
    }

    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            &format!("/runs/{foreign_run_id}/steer"),
            &run_tool_worker_token,
            &json!({ "text": "continue", "interrupt": false }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            &format!("/runs/{foreign_run_id}/questions/q-1/answer"),
            &run_tool_worker_token,
            &json!({ "kind": "yes" }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    // Self-access passes without any scope walk.
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{parent_run_id}/state"),
            &run_tool_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    // Creator allowance: a run created BY the worker stays manageable even
    // before any parent link exists — the sub-workflow pattern.
    let created_child = create_run_with_bearer(&app, &run_tool_worker_token).await;
    let projection = state
        .stores
        .runs
        .load_run_projection(&created_child)
        .await
        .unwrap()
        .expect("created run should have a projection");
    assert_eq!(projection.spec.provenance.subject, Principal::Worker {
        run_id: parent_run_id,
    },);

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{created_child}/state"),
            &run_tool_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    // The creator may link itself as the child's parent and unlink again;
    // false parenthood of foreign runs is denied by the same rule.
    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::PUT,
            &format!("/runs/{created_child}/parent"),
            &run_tool_worker_token,
            &json!({ "parent_id": parent_run_id.to_string() }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::DELETE,
            &format!("/runs/{created_child}/parent"),
            &run_tool_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    // Linking a FOREIGN run as parent is denied — that would mint read
    // access out of nothing.
    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::PUT,
            &format!("/runs/{foreign_run_id}/parent"),
            &run_tool_worker_token,
            &json!({ "parent_id": parent_run_id.to_string() }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    // Unknown run: 404 parity with the user path (not a scope 403).
    let unknown_run_id = RunId::new();
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{unknown_run_id}/state"),
            &run_tool_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;

    // inspects allowance: a worker declaring the target's workflow passes.
    let smoke_run_id = RunId::new();
    create_run_with_workflow_slug(&state, smoke_run_id, "smoke").await;
    let inspects_token = issue_test_inspects_worker_token(&RunId::new(), &["smoke".to_string()]);
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{smoke_run_id}/state"),
            &inspects_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    // ...while the same worker without the declaration is denied.
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{smoke_run_id}/state"),
            &run_tool_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;
}

#[tokio::test]
async fn agent_session_header_stamps_created_by_as_agent() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let intent =
        test_intent_with_bearer(&app, "workflow.fabro", MINIMAL_DOT, None, Some(&user_jwt)).await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api("/runs"))
                .header(header::AUTHORIZATION, format!("Bearer {user_jwt}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-fabro-agent-session", "01TESTAGENTSESSION01")
                .body(Body::from(intent.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::CREATED).await;
    assert_eq!(body["created_by"]["kind"], "agent");
    assert_eq!(body["created_by"]["session_id"], "01TESTAGENTSESSION01");
    let run_id: RunId = body["id"].as_str().unwrap().parse().unwrap();

    // The attribution survives the durable summary (GET run).
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(api(&format!("/runs/{run_id}")))
                .header(header::AUTHORIZATION, format!("Bearer {user_jwt}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["created_by"]["kind"], "agent");
    assert_eq!(body["created_by"]["session_id"], "01TESTAGENTSESSION01");
}

#[tokio::test]
async fn run_create_without_agent_session_header_keeps_user_created_by() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api("/runs"))
                .header(header::AUTHORIZATION, format!("Bearer {user_jwt}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    test_intent_with_bearer(
                        &app,
                        "workflow.fabro",
                        MINIMAL_DOT,
                        None,
                        Some(&user_jwt),
                    )
                    .await
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::CREATED).await;
    assert_eq!(body["created_by"]["kind"], "user");
    assert_eq!(body["created_by"]["login"], "octocat");
}

#[tokio::test]
async fn agent_session_header_grants_no_privilege_without_authentication() {
    let (_state, app) = jwt_auth_app();
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api("/runs"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-fabro-agent-session", "01TESTAGENTSESSION01")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(response, StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn worker_token_create_with_agent_session_header_keeps_worker_subject() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let origin_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_run_tools_worker_token(&origin_run_id);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api("/runs"))
                .header(header::AUTHORIZATION, format!("Bearer {worker_token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-fabro-agent-session", "01TESTAGENTSESSION01")
                .body(Body::from(
                    test_intent_with_bearer(
                        &app,
                        "workflow.fabro",
                        MINIMAL_DOT,
                        None,
                        Some(&user_jwt),
                    )
                    .await
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    // The marker only re-labels authenticated USER principals; a run-tools
    // worker creating a child keeps its worker attribution.
    let body = response_json!(response, StatusCode::CREATED).await;
    assert_eq!(body["created_by"]["kind"], "worker");
}

#[tokio::test]
async fn worker_token_controls_stage_artifact_route() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_worker_token(&run_id);
    let other_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let mismatched_worker_token = issue_test_worker_token(&other_run_id);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api(&format!(
                    "/runs/{run_id}/stages/code@2/artifacts?filename=artifact.txt&retry=1"
                )))
                .header(header::AUTHORIZATION, format!("Bearer {worker_token}"))
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from("artifact"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(response, StatusCode::NO_CONTENT).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api(&format!(
                    "/runs/{run_id}/stages/code@2/artifacts?filename=artifact.txt&retry=1"
                )))
                .header(header::AUTHORIZATION, format!("Bearer {user_jwt}"))
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from("artifact"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(response, StatusCode::NO_CONTENT).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api(&format!(
                    "/runs/{run_id}/stages/code@2/artifacts?filename=artifact.txt&retry=1"
                )))
                .header(
                    header::AUTHORIZATION,
                    format!("Bearer {mismatched_worker_token}"),
                )
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from("artifact"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api(&format!(
                    "/runs/{run_id}/stages/code@2/artifacts?filename=artifact.txt&retry=1"
                )))
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from("artifact"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(response, StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn worker_token_controls_command_log_route() {
    let (state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_worker_token(&run_id);
    let other_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let mismatched_worker_token = issue_test_worker_token(&other_run_id);
    let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
    workflow_event::append_event(
        &run_store,
        &run_id,
        &workflow_event::Event::CommandStarted {
            node_id:    "code".to_string(),
            script:     "echo hello".to_string(),
            command:    "echo hello".to_string(),
            language:   "shell".to_string(),
            timeout_ms: None,
        },
    )
    .await
    .unwrap();

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{run_id}/stages/code@1/logs/output"),
            &worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{run_id}/stages/code@1/logs/output"),
            &user_jwt,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{run_id}/stages/code@1/logs/output"),
            &mismatched_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(api(&format!("/runs/{run_id}/stages/code@1/logs/output")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(response, StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn worker_token_is_rejected_on_user_only_routes() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_worker_token(&run_id);
    let blob_hash = BlobHash::new(b"blob");
    let user_only_routes = vec![
        (Method::GET, "/runs".to_string()),
        (Method::POST, "/runs".to_string()),
        (Method::GET, "/runs/resolve".to_string()),
        (Method::POST, "/preflight".to_string()),
        (Method::POST, "/validate".to_string()),
        (Method::POST, "/graph/render".to_string()),
        (Method::GET, "/attach".to_string()),
        (Method::DELETE, format!("/runs/{run_id}")),
        (Method::GET, format!("/runs/{run_id}/attach")),
        (Method::GET, format!("/runs/{run_id}/checkpoint")),
        (Method::POST, format!("/runs/{run_id}/pause")),
        (Method::POST, format!("/runs/{run_id}/unpause")),
        (Method::GET, format!("/runs/{run_id}/graph")),
        (Method::GET, format!("/runs/{run_id}/graph/source")),
        (Method::GET, format!("/runs/{run_id}/stages")),
        (Method::GET, format!("/runs/{run_id}/artifacts")),
        (Method::GET, format!("/runs/{run_id}/artifacts/download")),
        (Method::GET, format!("/runs/{run_id}/files")),
        (
            Method::GET,
            format!("/runs/{run_id}/stages/code@2/artifacts"),
        ),
        (
            Method::GET,
            format!("/runs/{run_id}/stages/code@2/artifacts/download"),
        ),
        (Method::GET, format!("/runs/{run_id}/usage")),
        (Method::GET, format!("/runs/{run_id}/settings")),
        (Method::POST, format!("/runs/{run_id}/preview")),
        (Method::POST, format!("/runs/{run_id}/ssh")),
        (Method::GET, format!("/runs/{run_id}/sandbox/files")),
        (Method::GET, format!("/runs/{run_id}/sandbox/services")),
        (Method::GET, format!("/runs/{run_id}/sandbox/file")),
        (Method::PUT, format!("/runs/{run_id}/sandbox/file")),
    ];

    for (method, path) in user_only_routes {
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
            "{method} {path} unexpectedly accepted worker token with status {}",
            response.status()
        );
    }

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{run_id}/blobs/{blob_hash}"),
            &worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn batch_lifecycle_requires_user_authentication() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_worker_token(&run_id);
    let body = batch_lifecycle_body(&[run_id]);

    let unauthenticated = app
        .clone()
        .oneshot(json_request(Method::POST, "/runs/archive", &body))
        .await
        .unwrap();
    assert_status!(unauthenticated, StatusCode::UNAUTHORIZED).await;

    for path in ["/runs/archive", "/runs/unarchive"] {
        let worker_response = app
            .clone()
            .oneshot(json_bearer_request(
                Method::POST,
                path,
                &worker_token,
                &body,
            ))
            .await
            .unwrap();
        assert!(
            matches!(
                worker_response.status(),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            ),
            "{path} unexpectedly accepted worker token with status {}",
            worker_response.status()
        );
    }
}

#[tokio::test]
async fn batch_delete_requires_user_authentication() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_worker_token(&run_id);
    let body = batch_delete_body(&[run_id], false);

    let unauthenticated = app
        .clone()
        .oneshot(json_request(Method::POST, "/runs/delete", &body))
        .await
        .unwrap();
    assert_status!(unauthenticated, StatusCode::UNAUTHORIZED).await;

    let worker_response = app
        .oneshot(json_bearer_request(
            Method::POST,
            "/runs/delete",
            &worker_token,
            &body,
        ))
        .await
        .unwrap();
    assert!(
        matches!(
            worker_response.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ),
        "/runs/delete unexpectedly accepted worker token with status {}",
        worker_response.status()
    );
}
