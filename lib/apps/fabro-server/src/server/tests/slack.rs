use super::*;

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

#[tokio::test]
async fn slack_lifecycle_run_started_posts_for_matching_enabled_route() {
    let server = MockServer::start_async().await;
    let post = mock_slack_post(
        &server,
        vec![
            r##""channel":"#deploys""##.to_string(),
            "Fabro run started".to_string(),
            "Deploy workflow".to_string(),
            "Open in Fabro".to_string(),
        ],
        "100.1",
    )
    .await;
    let state = test_app_state();
    let service = slack_lifecycle_service(server.base_url(), None);
    let run_id = fixtures::RUN_1;
    let settings = workflow_settings_with_run_notifications(
        r##"
[run.notifications.deploys]
enabled = true
provider = "slack"
events = ["run.started", "run.completed", "run.failed"]

[run.notifications.deploys.slack]
channel = "#deploys"
"##,
        Some("Deploy workflow"),
    );
    let run_store =
        create_slack_notification_run(&state, run_id, settings, "deploy-graph", Some("deploy"))
            .await;
    let envelope =
        append_slack_notification_event(&run_store, run_id, &workflow_run_started_event(run_id))
            .await;

    service
        .handle_event(
            state.as_ref(),
            &envelope,
            Some("https://fabro.example/runs/run-1"),
        )
        .await;

    post.assert_async().await;
    assert!(
        service
            .posted_messages
            .lock()
            .expect("posted messages lock poisoned")
            .is_empty(),
        "lifecycle posts must not use interview message state"
    );
}

#[tokio::test]
async fn slack_lifecycle_interview_started_posts_question_to_route() {
    let server = MockServer::start_async().await;
    let post = mock_slack_post(
        &server,
        vec![
            r##""channel":"#gates""##.to_string(),
            "Should we ship the fix?".to_string(),
            "times out in ~5m".to_string(),
            "Open in Fabro".to_string(),
        ],
        "100.3",
    )
    .await;
    let state = test_app_state();
    let service = slack_lifecycle_service(server.base_url(), None);
    let run_id = fixtures::RUN_1;
    let settings = workflow_settings_with_run_notifications(
        r##"
[run.notifications.gates]
enabled = true
provider = "slack"
events = ["interview.started"]

[run.notifications.gates.slack]
channel = "#gates"
"##,
        Some("Revisor"),
    );
    let run_store =
        create_slack_notification_run(&state, run_id, settings, "revisor", Some("revisor")).await;
    let envelope = append_slack_notification_event(
        &run_store,
        run_id,
        &workflow_event::Event::InterviewStarted {
            question_id:     "q-1".to_string(),
            question:        "Should we ship the fix?".to_string(),
            stage:           "review".to_string(),
            question_type:   "yes_no".to_string(),
            options:         vec![],
            allow_freeform:  false,
            timeout_seconds: Some(300.0),
            context_display: None,
            review_target:   None,
        },
    )
    .await;

    service
        .handle_event(
            state.as_ref(),
            &envelope,
            Some("https://fabro.example/runs/run-1"),
        )
        .await;

    post.assert_async().await;
    assert_eq!(post.calls_async().await, 1, "must dispatch exactly once");
    assert!(
        service
            .posted_messages
            .lock()
            .expect("posted messages lock poisoned")
            .is_empty(),
        "route posts must not use interview bot message state"
    );
}

#[tokio::test]
async fn slack_lifecycle_interview_started_with_bot_and_route_posts_both_channels() {
    // A default channel AND a matching route both fire for the same
    // question: the bot owns its channel, the route owns its own —
    // including the (documented) case where both channels differ.
    let server = MockServer::start_async().await;
    let route_post = mock_slack_post(
        &server,
        vec![
            r##""channel":"#gates""##.to_string(),
            "Ship it?".to_string(),
        ],
        "100.5",
    )
    .await;
    let bot_post = mock_slack_post(
        &server,
        vec![
            r#""channel":"C-default""#.to_string(),
            "Ship it?".to_string(),
        ],
        "100.6",
    )
    .await;
    let state = test_app_state();
    // The bot posts to the default channel id resolved from the mock's
    // response; slack_lifecycle_service(base_url, default_channel).
    let service = slack_lifecycle_service(server.base_url(), Some("C-default"));
    let run_id = fixtures::RUN_1;
    let settings = workflow_settings_with_run_notifications(
        r##"
[run.notifications.gates]
enabled = true
provider = "slack"
events = ["interview.started"]

[run.notifications.gates.slack]
channel = "#gates"
"##,
        Some("Revisor"),
    );
    let run_store =
        create_slack_notification_run(&state, run_id, settings, "revisor", Some("revisor")).await;
    let envelope = append_slack_notification_event(
        &run_store,
        run_id,
        &workflow_event::Event::InterviewStarted {
            question_id:     "q-1".to_string(),
            question:        "Ship it?".to_string(),
            stage:           "review".to_string(),
            question_type:   "yes_no".to_string(),
            options:         vec![],
            allow_freeform:  false,
            timeout_seconds: None,
            context_display: None,
            review_target:   None,
        },
    )
    .await;

    service.handle_event(state.as_ref(), &envelope, None).await;

    route_post.assert_async().await;
    bot_post.assert_async().await;
    assert_eq!(route_post.calls_async().await, 1);
    assert_eq!(bot_post.calls_async().await, 1);
    // The bot, not the route, tracked its message for later updates.
    assert_eq!(
        service
            .posted_messages
            .lock()
            .expect("posted messages lock poisoned")
            .len(),
        1
    );
}

#[tokio::test]
async fn slack_lifecycle_interview_started_without_matching_route_does_not_post() {
    let server = MockServer::start_async().await;
    let post = mock_slack_post(
        &server,
        vec!["Should we ship the fix?".to_string()],
        "100.4",
    )
    .await;
    let state = test_app_state();
    let service = slack_lifecycle_service(server.base_url(), None);
    let run_id = fixtures::RUN_1;
    // Route subscribes only to terminal events: the pending question
    // must not reach it.
    let settings = workflow_settings_with_run_notifications(
        r##"
[run.notifications.deploys]
enabled = true
provider = "slack"
events = ["run.completed"]

[run.notifications.deploys.slack]
channel = "#deploys"
"##,
        Some("Deploy workflow"),
    );
    let run_store =
        create_slack_notification_run(&state, run_id, settings, "deploy", Some("deploy")).await;
    let envelope = append_slack_notification_event(
        &run_store,
        run_id,
        &workflow_event::Event::InterviewStarted {
            question_id:     "q-1".to_string(),
            question:        "Should we ship the fix?".to_string(),
            stage:           "review".to_string(),
            question_type:   "yes_no".to_string(),
            options:         vec![],
            allow_freeform:  false,
            timeout_seconds: None,
            context_display: None,
            review_target:   None,
        },
    )
    .await;

    service
        .handle_event(
            state.as_ref(),
            &envelope,
            Some("https://fabro.example/runs/run-1"),
        )
        .await;

    assert_eq!(
        post.calls_async().await,
        0,
        "no route matched interview.started, nothing may post"
    );
}

#[tokio::test]
async fn slack_lifecycle_run_completed_posts_result_and_duration() {
    let server = MockServer::start_async().await;
    let post = mock_slack_post(
        &server,
        vec![
            r##""channel":"#deploys""##.to_string(),
            "Fabro run completed".to_string(),
            "succeeded — completed".to_string(),
            "1m 5s".to_string(),
        ],
        "100.2",
    )
    .await;
    let state = test_app_state();
    let service = slack_lifecycle_service(server.base_url(), None);
    let run_id = fixtures::RUN_1;
    let settings = workflow_settings_with_run_notifications(
        r##"
[run.notifications.deploys]
enabled = true
provider = "slack"
events = ["run.completed"]

[run.notifications.deploys.slack]
channel = "#deploys"
"##,
        Some("Deploy workflow"),
    );
    let run_store = create_slack_notification_run(&state, run_id, settings, "deploy", None).await;
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunRunnable {
        source: fabro_types::RunRunnableSource::StartRequested,
        actor:  None,
    })
    .await
    .unwrap();
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunStarting)
        .await
        .unwrap();
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunRunning)
        .await
        .unwrap();
    let envelope = append_slack_notification_event(
        &run_store,
        run_id,
        &workflow_event::Event::WorkflowRunCompleted {
            timing:               fabro_types::RunTiming::wall_only(65_432),
            artifact_count:       0,
            status:               "succeeded".to_string(),
            reason:               SuccessReason::Completed,
            failure:              None,
            final_git_commit_sha: None,
            final_patch:          None,
            diff_summary:         None,
            usage:                None,
        },
    )
    .await;

    service.handle_event(state.as_ref(), &envelope, None).await;

    post.assert_async().await;
}

#[tokio::test]
async fn slack_lifecycle_run_failed_posts_failure_result_message_and_duration() {
    let server = MockServer::start_async().await;
    let post = mock_slack_post(
        &server,
        vec![
            r##""channel":"#deploys""##.to_string(),
            "Fabro run failed".to_string(),
            "workflow_error — command &lt;failed&gt; &amp; exited".to_string(),
            "1.2s".to_string(),
        ],
        "100.3",
    )
    .await;
    let state = test_app_state();
    let service = slack_lifecycle_service(server.base_url(), None);
    let run_id = fixtures::RUN_1;
    let settings = workflow_settings_with_run_notifications(
        r##"
[run.notifications.deploys]
enabled = true
provider = "slack"
events = ["run.failed"]

[run.notifications.deploys.slack]
channel = "#deploys"
"##,
        Some("Deploy workflow"),
    );
    let run_store = create_slack_notification_run(&state, run_id, settings, "deploy", None).await;
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunRunnable {
        source: fabro_types::RunRunnableSource::StartRequested,
        actor:  None,
    })
    .await
    .unwrap();
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunStarting)
        .await
        .unwrap();
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunRunning)
        .await
        .unwrap();
    let envelope = append_slack_notification_event(
        &run_store,
        run_id,
        &workflow_event::Event::WorkflowRunFailed {
            failure:              fabro_types::RunFailure {
                reason: fabro_types::FailureReason::WorkflowError,
                detail: FailureDetail::new(
                    "command <failed> & exited",
                    FailureCategory::Deterministic,
                ),
            },
            timing:               fabro_types::RunTiming::wall_only(1_234),
            final_git_commit_sha: None,
            final_patch:          None,
            diff_summary:         None,
            usage:                None,
        },
    )
    .await;

    service.handle_event(state.as_ref(), &envelope, None).await;

    post.assert_async().await;
}

#[tokio::test]
async fn slack_lifecycle_skips_non_matching_events_and_disabled_routes() {
    let server = MockServer::start_async().await;
    let unexpected = mock_slack_post(&server, Vec::new(), "100.4").await;
    let state = test_app_state();
    let service = slack_lifecycle_service(server.base_url(), None);
    let run_id = fixtures::RUN_1;
    let settings = workflow_settings_with_run_notifications(
        r##"
[run.notifications.disabled]
enabled = false
provider = "slack"
events = ["run.started"]

[run.notifications.disabled.slack]
channel = "#deploys"

[run.notifications.stage]
enabled = true
provider = "slack"
events = ["stage.completed"]

[run.notifications.stage.slack]
channel = "#deploys"
"##,
        Some("Deploy workflow"),
    );
    let run_store = create_slack_notification_run(&state, run_id, settings, "deploy", None).await;
    let envelope =
        append_slack_notification_event(&run_store, run_id, &workflow_run_started_event(run_id))
            .await;

    service.handle_event(state.as_ref(), &envelope, None).await;

    unexpected.assert_calls_async(0).await;
}

#[tokio::test]
async fn slack_lifecycle_missing_channel_is_skipped_without_blocking_other_routes() {
    let server = MockServer::start_async().await;
    let post = mock_slack_post(
        &server,
        vec![
            r##""channel":"#ops""##.to_string(),
            "Fabro run started".to_string(),
        ],
        "100.5",
    )
    .await;
    let state = test_app_state();
    let service = slack_lifecycle_service(server.base_url(), None);
    let run_id = fixtures::RUN_1;
    let settings = workflow_settings_with_run_notifications(
        r##"
[run.notifications.missing]
enabled = true
provider = "slack"
events = ["run.started"]

[run.notifications.unresolved]
enabled = true
provider = "slack"
events = ["run.started"]

[run.notifications.unresolved.slack]
channel = "{{ env.MISSING_SLACK_CHANNEL }}"

[run.notifications.valid]
enabled = true
provider = "slack"
events = ["run.started"]

[run.notifications.valid.slack]
channel = "#ops"
"##,
        Some("Deploy workflow"),
    );
    let run_store = create_slack_notification_run(&state, run_id, settings, "deploy", None).await;
    let envelope =
        append_slack_notification_event(&run_store, run_id, &workflow_run_started_event(run_id))
            .await;

    service.handle_event(state.as_ref(), &envelope, None).await;

    post.assert_async().await;
}

#[tokio::test]
async fn slack_lifecycle_uses_prior_pull_request_created_details() {
    let server = MockServer::start_async().await;
    let post = mock_slack_post(
        &server,
        vec![
            "Fabro run completed".to_string(),
            "https://github.com/fabro-sh/fabro/pull/42".to_string(),
            "#42".to_string(),
            "Ship &lt;prod&gt; &amp; notify".to_string(),
        ],
        "100.6",
    )
    .await;
    let state = test_app_state();
    let service = slack_lifecycle_service(server.base_url(), None);
    let run_id = fixtures::RUN_1;
    let settings = workflow_settings_with_run_notifications(
        r##"
[run.notifications.deploys]
enabled = true
provider = "slack"
events = ["run.completed"]

[run.notifications.deploys.slack]
channel = "#deploys"
"##,
        Some("Deploy workflow"),
    );
    let run_store = create_slack_notification_run(&state, run_id, settings, "deploy", None).await;
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunRunnable {
        source: fabro_types::RunRunnableSource::StartRequested,
        actor:  None,
    })
    .await
    .unwrap();
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunStarting)
        .await
        .unwrap();
    workflow_event::append_event(&run_store, &run_id, &workflow_event::Event::RunRunning)
        .await
        .unwrap();
    workflow_event::append_event(
        &run_store,
        &run_id,
        &workflow_event::Event::PullRequestCreated {
            pr_url:      "https://github.com/fabro-sh/fabro/pull/42".to_string(),
            pr_number:   42,
            owner:       "fabro-sh".to_string(),
            repo:        "fabro".to_string(),
            base_branch: "main".to_string(),
            head_branch: "fabro/run/test".to_string(),
            head_sha:    Some("final-sha".to_string()),
            title:       "Ship <prod> & notify".to_string(),
            draft:       false,
            auto_merge:  None,
        },
    )
    .await
    .unwrap();
    let envelope = append_slack_notification_event(
        &run_store,
        run_id,
        &workflow_event::Event::WorkflowRunCompleted {
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
    )
    .await;

    service.handle_event(state.as_ref(), &envelope, None).await;

    post.assert_async().await;
}

#[tokio::test]
async fn slack_interviews_keep_state_separate_from_lifecycle_notifications() {
    let server = MockServer::start_async().await;
    let interview_post = mock_slack_post(
        &server,
        vec![
            r##""channel":"#reviews""##.to_string(),
            "Answer deploy question".to_string(),
        ],
        "200.1",
    )
    .await;
    let lifecycle_post = mock_slack_post(
        &server,
        vec![
            r##""channel":"#deploys""##.to_string(),
            "Fabro run started".to_string(),
        ],
        "200.2",
    )
    .await;
    let state = test_app_state();
    let service = slack_lifecycle_service(server.base_url(), Some("#reviews"));
    let run_id = fixtures::RUN_1;
    let settings = workflow_settings_with_run_notifications(
        r##"
[run.notifications.deploys]
enabled = true
provider = "slack"
events = ["run.started"]

[run.notifications.deploys.slack]
channel = "#deploys"
"##,
        Some("Deploy workflow"),
    );
    let run_store = create_slack_notification_run(&state, run_id, settings, "deploy", None).await;
    let lifecycle_envelope =
        append_slack_notification_event(&run_store, run_id, &workflow_run_started_event(run_id))
            .await;
    let interview_envelope = append_slack_notification_event(
        &run_store,
        run_id,
        &workflow_event::Event::InterviewStarted {
            question_id:     "q-1".to_string(),
            question:        "Answer deploy question".to_string(),
            stage:           "review".to_string(),
            question_type:   "freeform".to_string(),
            options:         Vec::new(),
            allow_freeform:  true,
            timeout_seconds: None,
            context_display: None,
            review_target:   None,
        },
    )
    .await;

    service
        .handle_event(state.as_ref(), &lifecycle_envelope, None)
        .await;
    assert!(
        service
            .posted_messages
            .lock()
            .expect("posted messages lock poisoned")
            .is_empty(),
        "lifecycle notification should not record interview metadata"
    );
    assert!(
        service.thread_registry.resolve("200.2").is_none(),
        "lifecycle notification should not register answer threads"
    );

    service
        .handle_event(state.as_ref(), &interview_envelope, None)
        .await;

    lifecycle_post.assert_async().await;
    interview_post.assert_async().await;
    assert!(
        service
            .posted_messages
            .lock()
            .expect("posted messages lock poisoned")
            .contains_key(&(run_id, "q-1".to_string())),
        "interview posts should retain interview state"
    );
    assert!(
        service.thread_registry.resolve("200.1").is_some(),
        "freeform interview posts should register reply threads"
    );
}
