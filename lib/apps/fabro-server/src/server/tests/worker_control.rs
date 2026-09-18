use super::*;

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
async fn worker_control_stream_rejects_missing_user_and_cross_run_auth() {
    let (_state, app) = jwt_auth_app();
    let user_bearer = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_bearer).await;
    let worker_bearer = issue_test_worker_token(&run_id);
    let other_run_id = create_run_with_bearer(&app, &user_bearer).await;
    let other_worker_bearer = issue_test_worker_token(&other_run_id);
    let server = WorkerControlWsTestServer::spawn(app).await;

    assert_worker_control_ws_rejected(&server, run_id, None, None, StatusCode::UNAUTHORIZED).await;
    assert_worker_control_ws_rejected(
        &server,
        run_id,
        Some(&user_bearer),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;
    assert_worker_control_ws_rejected(
        &server,
        run_id,
        Some(&other_worker_bearer),
        None,
        StatusCode::FORBIDDEN,
    )
    .await;

    let mut socket = connect_worker_control_ws(&server, run_id, &worker_bearer, None).await;
    futures_util::SinkExt::send(&mut socket, WebSocketMessage::Close(None))
        .await
        .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn worker_control_stream_start_subscription_delivers_frames() {
    let (state, app) = jwt_auth_app();
    let user_bearer = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_bearer).await;
    let worker_bearer = issue_test_worker_token(&run_id);
    let server = WorkerControlWsTestServer::spawn(app).await;
    let mut socket = connect_worker_control_ws(&server, run_id, &worker_bearer, None).await;

    let expected = WorkerControlEnvelope::cancel_run();
    let id = state
        .worker_control_bus
        .publish(run_id, expected.clone())
        .await
        .unwrap();
    let frame = next_worker_control_frame(&mut socket).await;

    assert_eq!(frame.id, id.to_string());
    assert_eq!(frame.envelope, expected);
}

#[tokio::test(flavor = "current_thread")]
async fn worker_control_stream_after_subscription_delivers_only_later_frames() {
    let (state, app) = jwt_auth_app();
    let user_bearer = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_bearer).await;
    let worker_bearer = issue_test_worker_token(&run_id);
    let first = state
        .worker_control_bus
        .publish(run_id, WorkerControlEnvelope::cancel_run())
        .await
        .unwrap();
    let expected = WorkerControlEnvelope::pause_run();
    let second = state
        .worker_control_bus
        .publish(run_id, expected.clone())
        .await
        .unwrap();
    let server = WorkerControlWsTestServer::spawn(app).await;

    let mut socket =
        connect_worker_control_ws(&server, run_id, &worker_bearer, Some(first.as_str())).await;
    let frame = next_worker_control_frame(&mut socket).await;

    assert_eq!(frame.id, second.to_string());
    assert_eq!(frame.envelope, expected);
}

#[tokio::test(flavor = "current_thread")]
async fn worker_control_stream_invalid_cursor_is_http_gone_before_upgrade() {
    let (_state, app) = jwt_auth_app();
    let user_bearer = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_bearer).await;
    let worker_bearer = issue_test_worker_token(&run_id);
    let server = WorkerControlWsTestServer::spawn(app).await;

    assert_worker_control_ws_rejected(
        &server,
        run_id,
        Some(&worker_bearer),
        Some("local:999"),
        StatusCode::GONE,
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn worker_control_stream_rejects_missing_terminal_and_archived_runs() {
    let (state, app) = jwt_auth_app();
    let user_bearer = issue_test_user_jwt();
    let missing_run_id = RunId::new();
    let missing_worker_bearer = issue_test_worker_token(&missing_run_id);
    let terminal_run_id = RunId::new();
    create_succeeded_run(&state, terminal_run_id).await;
    let terminal_worker_bearer = issue_test_worker_token(&terminal_run_id);
    let archived_run_id = RunId::new();
    create_succeeded_run(&state, archived_run_id).await;
    let archived_worker_bearer = issue_test_worker_token(&archived_run_id);
    let archive_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{archived_run_id}/archive")))
                .header(header::AUTHORIZATION, format!("Bearer {user_bearer}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    response_json!(archive_response, StatusCode::OK).await;
    let server = WorkerControlWsTestServer::spawn(app).await;

    assert_worker_control_ws_rejected(
        &server,
        missing_run_id,
        Some(&missing_worker_bearer),
        None,
        StatusCode::NOT_FOUND,
    )
    .await;
    assert_worker_control_ws_rejected(
        &server,
        terminal_run_id,
        Some(&terminal_worker_bearer),
        None,
        StatusCode::CONFLICT,
    )
    .await;
    assert_worker_control_ws_rejected(
        &server,
        archived_run_id,
        Some(&archived_worker_bearer),
        None,
        StatusCode::CONFLICT,
    )
    .await;
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

#[test]
fn agent_fabro_tools_enabled_combines_run_flag_and_node_level_opt_in() {
    fn spec_with(flag: bool, node_fabro_tools: Option<&str>) -> RunSpec {
        let mut graph = Graph::new("develop");
        if let Some(fabro_tools) = node_fabro_tools {
            let mut planner = Node::new("planner");
            planner.attrs.insert(
                "fabro_tools".to_string(),
                AttrValue::String(fabro_tools.to_string()),
            );
            graph.nodes.insert("planner".to_string(), planner);
        }
        let mut settings = WorkflowSettings::default();
        settings.run.agent.fabro_tools = flag;
        RunSpec {
            run_id: RunId::new(),
            settings,
            graph,
            graph_source: None,
            workflow_slug: Some("develop".to_string()),
            workflow_version_id: None,
            target: None,
            automation: None,
            source_directory: None,
            git: None,
            labels: HashMap::new(),
            provenance: test_support::test_run_provenance(),
            definition_blob: None,
            spec_blob: None,
            fork_source_ref: None,
        }
    }

    // (a) fabro-c419: node-level fabro_tools with the run flag off still
    // provisions the fabro run tool services for the run.
    assert!(super::agent_fabro_tools_enabled(&spec_with(
        false,
        Some("fabro_runs_list")
    )));
    // (b) Neither the run flag nor a node-level opt-in: no services.
    assert!(!super::agent_fabro_tools_enabled(&spec_with(false, None)));
    // (c) Run-wide flag alone: unchanged, enabled.
    assert!(super::agent_fabro_tools_enabled(&spec_with(true, None)));
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
async fn worker_answer_transport_cancel_run_publishes_cancel_message() {
    let (transport, mut control_rx) = worker_transport_with_receiver(fixtures::RUN_1).await;

    transport.cancel_run().await.unwrap();

    assert_eq!(
        recv_worker_control_envelope(&mut control_rx).await,
        WorkerControlEnvelope::cancel_run()
    );
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
async fn worker_answer_transport_interrupt_publishes_interrupt_message() {
    let (transport, mut control_rx) = worker_transport_with_receiver(fixtures::RUN_1).await;
    let actor = Principal::System {
        system_kind: SystemActorKind::Engine,
    };

    transport.interrupt(actor.clone()).await.unwrap();

    assert_eq!(
        recv_worker_control_envelope(&mut control_rx).await,
        WorkerControlEnvelope::interrupt(actor)
    );
}

#[tokio::test]
async fn worker_answer_transport_interrupt_then_steer_publishes_single_combined_message() {
    let (transport, mut control_rx) = worker_transport_with_receiver(fixtures::RUN_1).await;
    let actor = Principal::System {
        system_kind: SystemActorKind::Engine,
    };

    transport
        .interrupt_then_steer("try again".to_string(), actor.clone())
        .await
        .unwrap();

    assert_eq!(
        recv_worker_control_envelope(&mut control_rx).await,
        WorkerControlEnvelope::interrupt_then_steer("try again", actor)
    );
}

#[tokio::test]
async fn worker_answer_transport_pair_commands_publish_control_messages() {
    let run_id = fixtures::RUN_1;
    let (transport, mut control_rx) = worker_transport_with_receiver(run_id).await;
    let pair_id = "01HZX6M29F1CD5YYMHT1F5D7WQ".parse().unwrap();
    let message_id = "01HZX6M4D7Y1QW0Q0P6V8Z4DR5".parse().unwrap();
    let actor = Principal::System {
        system_kind: SystemActorKind::Engine,
    };
    let target = pair_test_target();

    transport
        .start_pair(run_id, pair_id, target.clone(), actor.clone())
        .await
        .unwrap();
    transport
        .send_pair_message(
            pair_id,
            message_id,
            "inspect this".to_string(),
            Some("client-1".to_string()),
            actor.clone(),
        )
        .await
        .unwrap();
    transport.end_pair(pair_id, actor.clone()).await.unwrap();

    assert_eq!(
        recv_worker_control_envelope(&mut control_rx).await,
        WorkerControlEnvelope::start_pair(run_id, pair_id, target, actor.clone())
    );
    assert_eq!(
        recv_worker_control_envelope(&mut control_rx).await,
        WorkerControlEnvelope::pair_message(
            pair_id,
            message_id,
            "inspect this",
            Some("client-1".to_string()),
            actor.clone()
        )
    );
    assert_eq!(
        recv_worker_control_envelope(&mut control_rx).await,
        WorkerControlEnvelope::end_pair(pair_id, actor)
    );
}

#[tokio::test]
async fn worker_answer_transport_pause_and_unpause_publish_control_messages() {
    let (transport, mut control_rx) = worker_transport_with_receiver(fixtures::RUN_1).await;

    transport.pause_run().await.unwrap();
    transport.unpause_run().await.unwrap();

    assert_eq!(
        recv_worker_control_envelope(&mut control_rx).await,
        WorkerControlEnvelope::pause_run()
    );
    assert_eq!(
        recv_worker_control_envelope(&mut control_rx).await,
        WorkerControlEnvelope::unpause_run()
    );
}

#[tokio::test]
async fn in_process_answer_transport_cancel_run_cancels_pending_interviews() {
    let interviewer = Arc::new(ControlInterviewer::new());
    let emitter = Arc::new(fabro_workflow::event::Emitter::new(
        fabro_types::RunId::new(),
    ));
    let steering_hub = Arc::new(fabro_workflow::SteeringHub::new(emitter));
    let transport = RunAnswerTransport::InProcess {
        interviewer:  Arc::clone(&interviewer),
        steering_hub: Arc::clone(&steering_hub),
    };
    let mut question = Question::new("Approve?", QuestionType::YesNo);
    question.id = "q-1".to_string();
    let ask_interviewer = Arc::clone(&interviewer);
    let answer_task = tokio::spawn(async move { ask_interviewer.ask(question).await });
    tokio::task::yield_now().await;

    transport.cancel_run().await.unwrap();

    let answer = answer_task.await.unwrap().answer;
    assert_eq!(answer.value, AnswerValue::Cancelled);
}

#[tokio::test]
async fn run_tools_worker_start_pair_reaches_worker_control_domain_for_created_runs() {
    // fabro-4556: the worker-control domain probe keeps its semantics on
    // runs the worker created; foreign targets never reach the handler.
    let (state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let origin_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let foreign_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_run_tools_worker_token(&origin_run_id);

    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            &format!("/runs/{foreign_run_id}/pair"),
            &worker_token,
            &json!({ "stage_id": "ignored" }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    let target_run_id = create_run_with_bearer(&app, &worker_token).await;
    let target = pair_test_target();
    let _temp_dir = insert_running_control_run(&state, target_run_id, None);
    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        runs.get_mut(&target_run_id)
            .unwrap()
            .active_api_targets
            .insert(target.stage_id.clone(), target.clone());
    }

    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            &format!("/runs/{target_run_id}/pair"),
            &worker_token,
            &json!({ "stage_id": target.stage_id.to_string() }),
        ))
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::SERVICE_UNAVAILABLE).await;
    assert_eq!(body["errors"][0]["code"], "worker_control_unavailable");
}

#[tokio::test]
async fn cross_run_base_worker_remains_forbidden_from_pair_routes() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let origin_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let target_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_worker_token(&origin_run_id);

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{target_run_id}/pair"),
            &worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;
}

#[tokio::test]
async fn base_worker_token_is_rejected_by_run_tool_only_routes() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_worker_token(&run_id);

    for (method, path) in [
        (Method::GET, "/runs".to_string()),
        (Method::POST, "/runs".to_string()),
        (Method::GET, "/runs/resolve?selector=latest".to_string()),
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
            "{method} {path} unexpectedly accepted base worker token with status {}",
            response.status()
        );
    }
}

#[tokio::test]
async fn worker_started_child_run_requires_approval_before_becoming_runnable() {
    let (state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let parent_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_run_tools_worker_token(&parent_run_id);
    let mut child_intent =
        test_intent_with_bearer(&app, "workflow.fabro", MINIMAL_DOT, None, Some(&user_jwt)).await;
    child_intent["parent_id"] = json!(parent_run_id.to_string());

    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            "/runs",
            &worker_token,
            &child_intent,
        ))
        .await
        .unwrap();
    let child_body = response_json!(response, StatusCode::CREATED).await;
    let child_run_id = child_body["id"].as_str().unwrap().parse::<RunId>().unwrap();

    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            &format!("/runs/{child_run_id}/start"),
            &worker_token,
            &json!({ "resume": false }),
        ))
        .await
        .unwrap();
    let pending_body = response_json!(response, StatusCode::OK).await;
    assert_eq!(
        run_json_status(&pending_body),
        &json!({
            "kind": "pending",
            "reason": "approval_required"
        })
    );
    assert_eq!(
        pending_body["lifecycle"]["approval"]["state"].as_str(),
        Some("pending")
    );

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            "/system/info",
            &user_jwt,
            Body::empty(),
        ))
        .await
        .unwrap();
    let info_body = response_json!(response, StatusCode::OK).await;
    assert_eq!(info_body["runs"]["active"], 1);
    assert_eq!(info_body["runs"]["scheduler_slots_used"], 0);

    {
        let runs = state.runs.lock().expect("runs lock poisoned");
        assert_eq!(
            runs.get(&child_run_id).map(|run| run.status),
            Some(RunStatus::Pending {
                reason: fabro_types::PendingReason::ApprovalRequired,
            })
        );
    }

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::POST,
            &format!("/runs/{child_run_id}/approve"),
            &user_jwt,
            Body::empty(),
        ))
        .await
        .unwrap();
    let approved_body = response_json!(response, StatusCode::OK).await;
    assert_eq!(
        run_json_status(&approved_body),
        &json!({ "kind": "runnable" })
    );
    assert_eq!(
        approved_body["lifecycle"]["approval"]["state"].as_str(),
        Some("approved")
    );
    assert!(
        approved_body["lifecycle"]["approval"]["decided_at"]
            .as_str()
            .is_some()
    );

    let runs = state.runs.lock().expect("runs lock poisoned");
    assert_eq!(
        runs.get(&child_run_id).map(|run| run.status),
        Some(RunStatus::Runnable)
    );
}

#[tokio::test]
async fn worker_started_child_with_auto_approve_starts_runnable() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let parent_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_run_tools_worker_token(&parent_run_id);
    let mut child_intent = test_intent_with_bearer(
        &app,
        "workflow.fabro",
        MINIMAL_DOT,
        None,
        Some(&worker_token),
    )
    .await;
    child_intent["parent_id"] = json!(parent_run_id.to_string());
    child_intent["args"] = json!({ "auto_approve": true });

    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            "/runs",
            &worker_token,
            &child_intent,
        ))
        .await
        .unwrap();
    let child_body = response_json!(response, StatusCode::CREATED).await;
    let child_run_id = child_body["id"].as_str().unwrap().parse::<RunId>().unwrap();

    // fabro-2e3c: args.auto_approve maps to ApprovalMode::Auto, which opts
    // the parent-worker start out of the approval gate — orchestration
    // children start unattended instead of parking approval_required.
    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            &format!("/runs/{child_run_id}/start"),
            &worker_token,
            &json!({ "resume": false }),
        ))
        .await
        .unwrap();
    let started_body = response_json!(response, StatusCode::OK).await;
    assert_eq!(
        run_json_status(&started_body),
        &json!({ "kind": "runnable" })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_active_workers_uses_worker_runtime_for_live_refs() {
    let runtime = StdArc::new(RecordingWorkerRuntime::default());
    runtime.set_alive(true);
    let state = TestAppStateBuilder::new()
        .worker_runtime(runtime.clone())
        .build();
    let worker_refs = [test_worker_ref(u32::MAX - 1), test_worker_ref(u32::MAX)];
    let run_ids = [RunId::new(), RunId::new()];
    let temp_dir = tempfile::tempdir().unwrap();

    for (run_id, worker_ref) in run_ids.iter().zip(worker_refs.iter()) {
        create_durable_run_with_events(&state, *run_id, &[
            workflow_event::Event::RunSubmitted {
                definition_blob: None,
            },
            workflow_event::Event::RunStarting,
            workflow_event::Event::RunRunning,
        ])
        .await;

        let mut run = managed_run(
            String::new(),
            RunStatus::Running,
            chrono::Utc::now(),
            temp_dir.path().join(run_id.to_string()),
            RunExecutionMode::Start,
        );
        run.worker_ref = Some(worker_ref.clone());
        state
            .runs
            .lock()
            .expect("runs lock poisoned")
            .insert(*run_id, run);
    }

    let terminated = shutdown_active_workers_with_grace(
        &state,
        Duration::from_millis(0),
        Duration::from_millis(1),
    )
    .await
    .unwrap();

    assert_eq!(terminated, 2);
    assert_eq!(runtime.requested_refs().len(), 2);
    assert_eq!(runtime.forced_refs().len(), 2);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_active_workers_terminates_process_groups() {
    let state = test_app_state();
    let run_id = fixtures::RUN_4;

    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ])
    .await;

    let temp_dir = tempfile::tempdir().unwrap();
    let mut child = tokio::process::Command::new("sh");
    child
        .arg("-c")
        .arg("trap '' TERM; while :; do sleep 1; done")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    fabro_proc::pre_exec_setpgid(child.as_std_mut());
    let mut child = child.spawn().unwrap();
    let worker_process_id = child.id().expect("worker pid should be available");

    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let mut run = managed_run(
            String::new(),
            RunStatus::Running,
            chrono::Utc::now(),
            temp_dir.path().join(run_id.to_string()),
            RunExecutionMode::Start,
        );
        run.worker_ref = Some(test_worker_ref(worker_process_id));
        runs.insert(run_id, run);
    }

    let terminated = shutdown_active_workers_with_grace(
        &state,
        Duration::from_millis(50),
        Duration::from_millis(10),
    )
    .await
    .unwrap();
    assert_eq!(terminated, 1);

    let exit_status = tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .expect("worker should exit after shutdown")
        .expect("wait should succeed");
    assert!(!exit_status.success());
    assert!(!fabro_proc::process_group_alive(worker_process_id));

    let run_state = state
        .stores
        .runs
        .open_run_reader(&run_id)
        .await
        .unwrap()
        .state()
        .await
        .unwrap();
    let run_status = run_state.status;
    assert_eq!(run_status, RunStatus::Failed {
        reason: FailureReason::Terminated,
    });
}
