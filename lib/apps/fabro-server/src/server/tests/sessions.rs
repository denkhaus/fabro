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
async fn http_log_records_worker_principal_fields() {
    let (_state, app) = jwt_auth_app();
    let user_bearer = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_bearer).await;
    let worker_bearer = issue_test_worker_token(&run_id);
    let (_guard, events) = capture_server_logs();

    let response = app
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{run_id}/state"),
            &worker_bearer,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let events = events.lock().expect("captured log events").clone();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_log_field(event, "principal_kind", "worker");
    assert_log_field(event, "auth_status", "authenticated");
    assert_log_field(event, "run_id", &run_id.to_string());
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
async fn worker_answer_transport_steer_publishes_plain_steer_message() {
    let (transport, mut control_rx) = worker_transport_with_receiver(fixtures::RUN_1).await;
    let actor = Principal::System {
        system_kind: SystemActorKind::Engine,
    };

    transport
        .steer("try again".to_string(), actor.clone())
        .await
        .unwrap();

    assert_eq!(
        recv_worker_control_envelope(&mut control_rx).await,
        WorkerControlEnvelope::steer("try again", actor)
    );
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
async fn get_questions_returns_empty_list() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    // Start a run
    let req = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(intent_body(&app, MINIMAL_DOT).await)
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    let run_id = body["id"].as_str().unwrap().parse::<RunId>().unwrap();

    // Get questions (should be empty for a run without wait.human nodes)
    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/questions")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert!(body["data"].is_array());
    assert_eq!(body["meta"]["has_more"], false);
}

#[test]
fn validate_answer_for_question_accepts_no_for_confirmation() {
    let question = InterviewQuestionRecord {
        id:              "q-1".to_string(),
        text:            "Continue?".to_string(),
        stage:           "gate".to_string(),
        question_type:   QuestionType::Confirmation,
        options:         vec![],
        allow_freeform:  false,
        timeout_seconds: None,
        context_display: None,
        review_target:   None,
    };

    let result = validate_answer_for_question(&question, &Answer::no());

    assert!(result.is_ok());
}

#[test]
fn answer_from_typed_yes_request_maps_to_yes_answer() {
    let question = InterviewQuestionRecord {
        id:              "q-1".to_string(),
        text:            "Continue?".to_string(),
        stage:           "gate".to_string(),
        question_type:   QuestionType::YesNo,
        options:         vec![],
        allow_freeform:  false,
        timeout_seconds: None,
        context_display: None,
        review_target:   None,
    };
    let req: SubmitAnswerRequest = serde_json::from_value(json!({ "kind": "yes" })).unwrap();

    let answer = answer_from_request(req, &question).unwrap();

    assert_eq!(answer.value, AnswerValue::Yes);
}

#[test]
fn answer_from_typed_no_request_maps_to_no_answer() {
    let question = InterviewQuestionRecord {
        id:              "q-1".to_string(),
        text:            "Continue?".to_string(),
        stage:           "gate".to_string(),
        question_type:   QuestionType::YesNo,
        options:         vec![],
        allow_freeform:  false,
        timeout_seconds: None,
        context_display: None,
        review_target:   None,
    };
    let req: SubmitAnswerRequest = serde_json::from_value(json!({ "kind": "no" })).unwrap();

    let answer = answer_from_request(req, &question).unwrap();

    assert_eq!(answer.value, AnswerValue::No);
}

#[test]
fn answer_from_typed_selected_request_validates_and_attaches_option() {
    let question = InterviewQuestionRecord {
        id:              "q-1".to_string(),
        text:            "Choose one.".to_string(),
        stage:           "gate".to_string(),
        question_type:   QuestionType::MultipleChoice,
        options:         vec![fabro_types::run_event::InterviewOption {
            key:         "approve".to_string(),
            label:       "Approve".to_string(),
            description: None,
            preview:     None,
        }],
        allow_freeform:  false,
        timeout_seconds: None,
        context_display: None,
        review_target:   None,
    };
    let req: SubmitAnswerRequest =
        serde_json::from_value(json!({ "kind": "selected", "option_key": "approve" })).unwrap();

    let answer = answer_from_request(req, &question).unwrap();

    assert_eq!(answer.value, AnswerValue::Selected("approve".to_string()));
    assert_eq!(
        answer
            .selected_option
            .as_ref()
            .map(|option| option.label.as_str()),
        Some("Approve")
    );
}

#[test]
fn answer_from_typed_multi_selected_request_validates_option_keys() {
    let question = InterviewQuestionRecord {
        id:              "q-1".to_string(),
        text:            "Choose many.".to_string(),
        stage:           "gate".to_string(),
        question_type:   QuestionType::MultiSelect,
        options:         vec![
            fabro_types::run_event::InterviewOption {
                key:         "approve".to_string(),
                label:       "Approve".to_string(),
                description: None,
                preview:     None,
            },
            fabro_types::run_event::InterviewOption {
                key:         "notify".to_string(),
                label:       "Notify".to_string(),
                description: None,
                preview:     None,
            },
        ],
        allow_freeform:  false,
        timeout_seconds: None,
        context_display: None,
        review_target:   None,
    };
    let req: SubmitAnswerRequest = serde_json::from_value(json!({
        "kind": "multi_selected",
        "option_keys": ["approve", "notify"],
    }))
    .unwrap();

    let answer = answer_from_request(req, &question).unwrap();

    assert_eq!(
        answer.value,
        AnswerValue::MultiSelected(vec!["approve".to_string(), "notify".to_string()])
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
async fn fabro_ask_proves_the_wire_path_and_analyst_sentinels() {
    use fabro_client::{Client as ServerClient, Credential as ClientCredential, ServerTarget};
    use fabro_tool::{FabroAskParams, ValidatedAsk, ask_run};

    let sandbox_dir = tempfile::tempdir().unwrap();
    let sandbox_path = sandbox_dir.path().to_path_buf();

    // Scripted LLM: the strict mock carries the ANALYST SESSION sentinels
    // as matchers — positive (resolved model id + the five policy-allowed
    // tools) and negative (no write/shell tools reach the provider).
    let mock_llm = httpmock::MockServer::start();
    let state = crate::test_support::jwt_auth_state_with_openai_base_url(
        &mock_llm.base_url(),
        TEST_SESSION_SECRET,
    );
    // Resolve the model the way the server does for a session without an
    // explicit pick: the default among providers with credentials (only
    // openai has a vault key here).
    let eligible = state
        .resolve_llm_client()
        .await
        .expect("test LLM client should resolve")
        .provider_ids()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let expected_model =
        fabro_llm::selection::resolve_selection(state.catalog().as_ref(), None, None, &eligible)
            .expect("a default model should resolve")
            .model;
    let llm_post = mock_llm.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .body_includes(format!(r#""model":"{expected_model}""#))
            .body_includes("fabro_run_events")
            .body_includes("fabro_run_get")
            // The native read tools serialize under the profile's own
            // vocabulary ("read…" family), not the canonical names.
            .body_includes("read")
            // Negative sentinels: quoted names must not appear anywhere —
            // the ask-fabro policy denies every write/execute tool.
            .body_excludes(r#""write_file""#)
            .body_excludes(r#""shell""#)
            .body_excludes(r#""spawn_agent""#);
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(
                [
                    r#"data: {"type":"response.output_text.delta","delta":"The gate timed out waiting for approval."}"#,
                    "\n\n",
                    r#"data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","output":[],"usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}}}"#,
                    "\n\n",
                ]
                .concat(),
            );
    });

    // Spawn the real router on a socket so the worker-credentialed
    // fabro_client can reach it; the daemon record points the analyst's
    // own API client at the same server.
    let app = build_router(Arc::clone(&state), jwt_auth_mode());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have a local address");
    let runtime_directory =
        fabro_config::Storage::new(state.server_storage_dir()).runtime_directory();
    ServerDaemon::new(
        std::process::id(),
        Bind::Tcp(addr),
        runtime_directory.log_path(),
    )
    .write(&runtime_directory)
    .unwrap();
    let server_task = tokio::spawn(async move {
        let result = axum::serve(listener, app.into_make_service()).await;
        if let Err(err) = result {
            tracing::debug!(error = %err, "test ask server stopped");
        }
    });

    // Terminal target runs: develop (in scope) and nightly (out of scope).
    let terminal_run_with_slug = |slug: &'static str| {
        let state = Arc::clone(&state);
        let sandbox_path = sandbox_path.clone();
        async move {
            let run_id = RunId::new();
            let run_store = state.stores.runs.create_run(&run_id).await.unwrap();
            for event in [
                workflow_event::Event::RunCreated {
                    run_id,
                    title: None,
                    settings: serde_json::to_value(fabro_types::WorkflowSettings::default())
                        .unwrap(),
                    graph: serde_json::to_value(Graph::new(slug)).unwrap(),
                    workflow_source: None,
                    labels: std::collections::BTreeMap::default(),
                    source_directory: None,
                    workflow_slug: Some(slug.to_string()),
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
                workflow_event::Event::SandboxInitialized {
                    provider:          SandboxProviderKind::LOCAL,
                    // The Host provider attaches only to real directories
                    // whose canonical path matches the id it derives from
                    // them, so the record uses a real temp dir.
                    id:                fabro_sandbox::test_support::local_sandbox_id(&sandbox_path)
                        .await,
                    working_directory: sandbox_path.to_string_lossy().to_string(),
                    image:             None,
                    snapshot:          None,
                    repo_cloned:       None,
                    clone_origin_url:  None,
                    clone_branch:      None,
                    workspace_root:    None,
                    repos_root:        None,
                    primary_repo_path: None,
                    primary_repo_link: None,
                },
                workflow_event::Event::RunPending {
                    reason: fabro_types::PendingReason::ApprovalRequired,
                    actor:  None,
                },
                workflow_event::Event::RunRunnable {
                    source: fabro_types::RunRunnableSource::StartRequested,
                    actor:  None,
                },
                workflow_event::Event::RunStarting,
                workflow_event::Event::RunRunning,
                workflow_event::Event::WorkflowRunCompleted {
                    timing:               fabro_types::RunTiming::default(),
                    artifact_count:       0,
                    status:               "succeeded".to_string(),
                    reason:               fabro_types::SuccessReason::Completed,
                    failure:              None,
                    final_git_commit_sha: None,
                    final_patch:          None,
                    diff_summary:         None,
                    usage:                None,
                },
            ] {
                workflow_event::append_event(&run_store, &run_id, &event)
                    .await
                    .unwrap();
            }
            run_id
        }
    };
    let develop_run_id = Box::pin(terminal_run_with_slug("develop")).await;
    let nightly_run_id = Box::pin(terminal_run_with_slug("nightly")).await;

    // Worker with inspects=["develop"] reaching the server over the wire.
    let worker_token = issue_test_inspects_worker_token(&RunId::new(), &["develop".to_string()]);
    let target: ServerTarget = format!("http://{addr}")
        .parse()
        .expect("test server target should parse");
    let api_client = ServerClient::builder()
        .target(target)
        .credential(ClientCredential::Worker(worker_token))
        .connect()
        .await
        .expect("worker client should connect");
    let backend = std::sync::Arc::new(fabro_tool::fabro_client::ClientBackend::new(
        std::sync::Arc::new(api_client),
    ));

    // Probe A: a user JWT through the same socket.
    let user_client = ServerClient::builder()
        .target(format!("http://{addr}").parse::<ServerTarget>().unwrap())
        .credential(ClientCredential::DevToken(issue_test_user_jwt()))
        .connect()
        .await
        .expect("user client should connect");
    user_client
        .resolve_run(&develop_run_id.to_string())
        .await
        .expect("PROBE A: user jwt resolve should work");

    // Worker credential sanity on the wire before the ask chain.
    fabro_tool::FabroToolBackend::resolve_run(backend.as_ref(), &develop_run_id.to_string())
        .await
        .expect("worker resolve should succeed");

    // (a) In-scope ask: the full chain runs — session create over HTTP,
    // turn streamed over SSE, analyst answer returned to the caller.
    let result = ask_run(
        std::sync::Arc::clone(&backend) as std::sync::Arc<dyn fabro_tool::FabroToolBackend>,
        ValidatedAsk::try_from(FabroAskParams {
            run_id:   develop_run_id.to_string(),
            question: "Why did the gate time out?".to_string(),
        })
        .unwrap(),
        &["develop".to_string()],
    )
    .await
    .expect("in-scope ask should succeed through the real wire");
    assert_eq!(result.answer, "The gate timed out waiting for approval.");
    // The strict mock carried every sentinel (positive and negative); a
    // hit proves the wire request satisfied all of them.
    assert_eq!(llm_post.calls(), 1, "exactly one analyst turn");

    // (b) Out-of-scope target: the tool rejects before the round trip.
    let err = ask_run(
        std::sync::Arc::clone(&backend) as std::sync::Arc<dyn fabro_tool::FabroToolBackend>,
        ValidatedAsk::try_from(FabroAskParams {
            run_id:   nightly_run_id.to_string(),
            question: "Why did this run fail?".to_string(),
        })
        .unwrap(),
        &["develop".to_string()],
    )
    .await
    .expect_err("out-of-scope targets must be rejected");
    assert!(err.as_str().contains("inspects scope"), "{err}");

    // (c) Wire-level authz: session creation on the out-of-scope run is
    // rejected BY THE SERVER (403), bypassing the tool-side check.
    let direct_client = ServerClient::builder()
        .target(
            format!("http://{addr}")
                .parse::<ServerTarget>()
                .expect("target should parse"),
        )
        .credential(ClientCredential::Worker(issue_test_inspects_worker_token(
            &RunId::new(),
            &["develop".to_string()],
        )))
        .connect()
        .await
        .expect("direct client should connect");
    let rejected = direct_client
        .create_run_session(nightly_run_id, fabro_api::types::CreateRunSessionRequest {
            title:    Some("revisor question".to_string()),
            model:    None,
            provider: None,
        })
        .await
        .expect_err("server must reject out-of-scope session creation");
    assert!(
        rejected.to_string().contains("Access denied"),
        "expected the server's 403 body on the wire, got: {rejected}"
    );

    server_task.abort();
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
async fn steer_without_active_steerable_session_forwards_plain_steer_for_buffering() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = fixtures::RUN_1;
    let (transport, mut control_rx) = worker_transport_with_receiver(run_id).await;
    let _temp_dir = insert_running_control_run(&state, run_id, Some(transport));

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/steer")))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"text":"try again"}"#))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::ACCEPTED).await;
    let envelope = recv_worker_control_envelope(&mut control_rx).await;
    assert!(matches!(
        envelope.message,
        WorkerControlMessage::Steer { ref text, .. } if text == "try again"
    ));
}

#[tokio::test]
async fn steer_with_active_non_steerable_session_returns_conflict() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = fixtures::RUN_1;
    let stage_id = StageId::new("agent", 1);
    let (transport, _control_rx) = worker_transport_with_receiver(run_id).await;
    let _temp_dir = insert_running_control_run(&state, run_id, Some(transport));
    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        runs.get_mut(&run_id)
            .unwrap()
            .active_non_steerable_stages
            .insert(stage_id, "session-a".to_string());
    }

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/steer")))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"text":"try again"}"#))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = body_json(response.into_body()).await;
    assert_eq!(body["errors"][0]["code"], "agent_not_steerable");
}

#[tokio::test]
async fn steer_with_active_acp_session_forwards_to_worker() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = fixtures::RUN_1;
    let (transport, mut control_rx) = worker_transport_with_receiver(run_id).await;
    let _temp_dir = insert_running_control_run(&state, run_id, Some(transport));

    let started = acp_event_for_stage(&run_id, &workflow_event::Event::AgentAcpStarted {
        node_id:     "agent".to_string(),
        visit:       1,
        command:     "python fake_agent.py".to_string(),
        config_name: None,
    });
    update_live_run_from_event(&state, run_id, &started);
    let activated =
        workflow_event::to_run_event(&run_id, &workflow_event::Event::AgentSessionActivated {
            node_id:          "agent".to_string(),
            visit:            1,
            session_id:       "acp-session".to_string(),
            thread_id:        None,
            provider:         Some(AgentBackend::Acp.to_string()),
            model:            None,
            reasoning_effort: None,
            speed:            None,
            permission_level: None,
            capabilities:     vec![SessionCapability::Steer],
        });
    update_live_run_from_event(&state, run_id, &activated);

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/steer")))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"text":"try again"}"#))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::ACCEPTED).await;
    let envelope = recv_worker_control_envelope(&mut control_rx).await;
    assert!(matches!(
        envelope.message,
        WorkerControlMessage::Steer { ref text, .. } if text == "try again"
    ));
}

#[tokio::test]
async fn attach_stream_replays_agent_message_reasoning() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = fixtures::RUN_1;

    create_durable_run_with_events(&state, run_id, &[
        stage_started_event("code", "agent"),
        agent_message_event(
            "code",
            1,
            "session-1",
            "",
            None,
            Some(ReasoningOutput::new(
                "inspect the sink first",
                "read events.rs, then attach",
            )),
        ),
        workflow_event::Event::WorkflowRunCompleted {
            timing:               fabro_types::RunTiming::wall_only(1000),
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
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/attach?since_seq=1")))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_bytes!(response, StatusCode::OK).await;
    let text = String::from_utf8(body.clone()).unwrap();

    let message = text
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str::<serde_json::Value>(data).ok())
        .find(|value| value["event"] == "agent.message")
        .expect("attach stream should replay the agent message");
    let reasoning = &message["properties"]["event"]["AssistantMessage"]["reasoning"];
    assert_eq!(reasoning["summary"], "inspect the sink first");
    assert_eq!(reasoning["trace"], "read events.rs, then attach");
}
