use super::*;

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
async fn subprocess_pre_start_failure_persists_launch_failure_from_runnable() {
    assert_subprocess_pre_start_failure(
        PreStartWorkerOutcome::LaunchFailure,
        FailureReason::LaunchFailed,
    )
    .await;
}

#[tokio::test]
async fn subprocess_pre_start_failure_persists_early_worker_exit_from_runnable() {
    assert_subprocess_pre_start_failure(
        PreStartWorkerOutcome::EarlyExit,
        FailureReason::Terminated,
    )
    .await;
}

#[tokio::test]
async fn subprocess_pre_start_failure_preserves_pending_cancellation() {
    let runtime = StdArc::new(PreStartWorkerRuntime::held(
        PreStartWorkerOutcome::LaunchFailure,
    ));
    let state = subprocess_pre_start_failure_state(StdArc::clone(&runtime));
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_and_start_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .expect("created run id should parse");

    let execution = tokio::spawn(execute_run(Arc::clone(&state), run_id));
    runtime.wait_for_start().await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/cancel")))
                .body(Body::empty())
                .expect("cancel request should build"),
        )
        .await
        .expect("cancel request should complete");
    assert_status!(response, StatusCode::ACCEPTED).await;

    runtime.release_held_start();
    execution.await.expect("run execution task should complete");

    assert_eq!(runtime.start_count(), 1);
    assert_run_failed_before_start(&state, run_id, FailureReason::Cancelled).await;
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

#[tokio::test]
async fn delete_terminal_managed_run_does_not_send_cancel_signal() {
    let state = test_app_state();
    let run_id = fixtures::RUN_1;
    create_durable_run_with_events(&state, run_id, &[
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

    let temp = tempfile::tempdir().unwrap();
    let run_dir = temp.path().join("run");
    std::fs::create_dir_all(&run_dir).unwrap();
    let cancel_token = CancellationToken::new();
    let mut run = managed_run(
        MINIMAL_DOT.to_string(),
        RunStatus::Running,
        Utc::now(),
        run_dir,
        RunExecutionMode::Start,
    );
    run.cancel_token = Some(cancel_token.clone());
    let (cancel_tx, _cancel_rx) = oneshot::channel();
    run.cancel_tx = Some(cancel_tx);
    state
        .runs
        .lock()
        .expect("runs lock poisoned")
        .insert(run_id, run);

    delete_run_internal(state.as_ref(), run_id, true)
        .await
        .unwrap();

    assert!(!cancel_token.is_cancelled());
}

/// Append a stage lifecycle event with an explicit `StageScope`, so the
/// stored envelope carries the full `stage_id` (`node_id@visit`). The bare
/// [`workflow_event::append_event`] helper only writes `node_id` because
/// stage lifecycle variants don't carry visit in their payload — production
/// always emits via `Emitter::emit_scoped`.

#[tokio::test]
async fn run_usage_sums_usage_across_retry_visits_and_uses_latest_model() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();

    create_priced_retry_run(&state, run_id).await;
    let success_usage = test_priced_usage("gpt-new", 200, 20);
    let mut latest_outcome: Outcome<Option<fabro_types::ModelUsage>> = Outcome::success();
    latest_outcome.usage = Some(success_usage);
    latest_outcome.timing = Some(fabro_types::StageTiming::wall_only(800));
    let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
    workflow_event::append_event(
        &run_store,
        &run_id,
        &workflow_event::Event::CheckpointCompleted {
            graph_visit: None,
            resumed_from_stage_id: None,
            node_id: "verify".to_string(),
            status: "running".to_string(),
            current_node: "verify".to_string(),
            completed_nodes: vec!["verify".to_string(), "verify".to_string()],
            node_retries: std::collections::BTreeMap::from([("verify".to_string(), 2)]),
            context_values: std::collections::BTreeMap::new(),
            node_outcomes: std::collections::BTreeMap::from([(
                "verify".to_string(),
                latest_outcome,
            )]),
            next_node_id: None,
            git_commit_sha: None,
            loop_failure_signatures: std::collections::BTreeMap::new(),
            restart_failure_signatures: std::collections::BTreeMap::new(),
            node_visits: std::collections::BTreeMap::from([("verify".to_string(), 2usize)]),
            diff: None,
            diff_summary: None,
        },
    )
    .await
    .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/usage")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;

    let stages = body["stages"].as_array().unwrap();
    assert_eq!(stages.len(), 1);
    assert_eq!(stages[0]["stage"]["id"], "verify");
    assert_eq!(stages[0]["model"]["provider"], "openai");
    assert_eq!(stages[0]["model"]["model_id"], "gpt-new");
    assert_eq!(stages[0]["usage"]["tokens"]["input"], 300);
    assert_eq!(stages[0]["usage"]["tokens"]["output"], 30);
    assert_eq!(stages[0]["usage"]["cost"]["usd_micros"], 330);
    assert!(stages[0]["timing"]["wall_time_ms"].as_u64().unwrap() == 2000);

    assert_eq!(body["totals"]["usage"]["tokens"]["input"], 300);
    assert_eq!(body["totals"]["usage"]["tokens"]["output"], 30);
    assert_eq!(body["totals"]["usage"]["cost"]["usd_micros"], 330);
    assert!(body["totals"]["timing"]["wall_time_ms"].as_u64().unwrap() == 2000);

    let by_model = body["by_model"].as_array().unwrap();
    assert_eq!(by_model.len(), 2);
    let old_model = by_model
        .iter()
        .find(|entry| entry["model"]["model_id"] == "gpt-old")
        .unwrap();
    let new_model = by_model
        .iter()
        .find(|entry| entry["model"]["model_id"] == "gpt-new")
        .unwrap();
    assert_eq!(old_model["model"]["provider"], "openai");
    assert_eq!(new_model["model"]["provider"], "openai");
    assert_eq!(old_model["stages"], 1);
    assert_eq!(old_model["usage"]["tokens"]["input"], 100);
    assert_eq!(new_model["stages"], 1);
    assert_eq!(new_model["usage"]["tokens"]["input"], 200);
}

/// The stage popover reads `usage` off the stages list, so it must be scoped
/// to one visit — unlike the Usage tab's rows, which sum every visit of a
/// node.

#[tokio::test]
async fn list_run_stages_shows_retrying_when_failed_will_retry() {
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
        "work",
        1,
        &workflow_event::Event::StageStarted {
            graph_visit:           None,
            resumed_from_stage_id: None,
            node_id:               "work".to_string(),
            name:                  "Work".to_string(),
            index:                 0,
            handler_type:          "command".to_string(),
            attempt:               1,
            max_attempts:          3,
        },
    )
    .await;
    // Only StageFailed, no StageRetrying yet — should still render retrying
    // because props.will_retry is true.
    append_scoped_stage_event(
        &state,
        run_id,
        "work",
        1,
        &workflow_event::Event::StageFailed {
            node_id:        "work".to_string(),
            name:           "Work".to_string(),
            index:          0,
            failure:        FailureDetail::new("flake", FailureCategory::TransientInfra),
            will_retry:     true,
            timing:         fabro_types::StageTiming::wall_only(5),
            usage_by_model: Vec::new(),
            usage:          None,
            actor:          None,
        },
    )
    .await;

    let response = app
        .clone()
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
    assert_eq!(stage_status(&body, "work@1"), "retrying");
}

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

/// ADR-0019.6 (fabro-c274): a Worker-principal run whose config declares
/// GitHub permissions must NOT depend on server-side GitHub credentials.
/// With no GITHUB_TOKEN in the vault the run starts, succeeds, and executes
/// nodes credential-free instead of hard-failing at credential resolution.

#[tokio::test]
async fn submit_answer_not_found_run() {
    let app = test_app_with();
    let missing_run_id = fixtures::RUN_64;

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{missing_run_id}/questions/q1/answer")))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({"kind": "yes"})).unwrap(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn submit_pending_interview_answer_rejects_invalid_answer_shape() {
    let state = test_app_state();
    let pending = LoadedPendingInterview {
        run_id:   fixtures::RUN_1,
        qid:      "q-1".to_string(),
        question: InterviewQuestionRecord {
            id:              "q-1".to_string(),
            text:            "Approve deploy?".to_string(),
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
        },
    };

    let response = submit_pending_interview_answer(
        state.as_ref(),
        &pending,
        AnswerSubmission::system(
            Answer::text("not a valid multiple choice answer"),
            SystemActorKind::Engine,
        ),
    )
    .await
    .unwrap_err();

    assert_status!(response, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn start_run_transitions_to_runnable() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    // Create a run
    let req = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(intent_body(&app, MINIMAL_DOT).await)
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    let run_id = body["id"].as_str().unwrap();

    // Start it
    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/start")))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(run_json_status(&body)["kind"], "runnable");
    assert_eq!(body["title"], "Test");

    let status = state
        .stores
        .runs
        .open_run_reader(&run_id.parse::<RunId>().unwrap())
        .await
        .unwrap()
        .state()
        .await
        .unwrap()
        .status;
    assert_eq!(status, RunStatus::Runnable);
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

#[tokio::test]
async fn denying_pending_child_run_fails_with_approval_denied() {
    let (_state, app) = jwt_auth_app();
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
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            &format!("/runs/{child_run_id}/deny"),
            &user_jwt,
            &json!({ "reason": "  " }),
        ))
        .await
        .unwrap();
    let denied_body = response_json!(response, StatusCode::OK).await;
    assert_eq!(
        run_json_status(&denied_body),
        &json!({
            "kind": "failed",
            "reason": "approval_denied"
        })
    );
    assert_eq!(
        denied_body["lifecycle"]["approval"]["state"].as_str(),
        Some("denied")
    );
    assert!(denied_body["lifecycle"]["approval"]["denial_reason"].is_null());
}

#[tokio::test]
async fn start_run_conflict_when_not_submitted() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    // Create a run
    let req = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(intent_body(&app, MINIMAL_DOT).await)
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    let run_id = body["id"].as_str().unwrap();

    // Start it (transitions to runnable)
    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/start")))
        .body(Body::empty())
        .unwrap();
    app.clone().oneshot(req).await.unwrap();

    // Start it again — should 409
    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/start")))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::CONFLICT).await;
}

#[tokio::test]
async fn resume_cancelled_run_with_checkpoint_transitions_to_runnable() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    let checkpoint = Checkpoint::from_context(
        &fabro_workflow::context::Context::new(),
        "start",
        vec!["start".to_string()],
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        Some("exit".to_string()),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
    );
    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunRunnable {
            source: fabro_types::RunRunnableSource::StartRequested,
            actor:  None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
        workflow_event::Event::CheckpointCompleted {
            graph_visit: None,
            resumed_from_stage_id: None,
            node_id: checkpoint.current_node.clone(),
            status: "succeeded".to_string(),
            current_node: checkpoint.current_node.clone(),
            completed_nodes: checkpoint.completed_nodes.clone(),
            node_retries: checkpoint.node_retries.clone().into_iter().collect(),
            context_values: checkpoint.context_values.clone().into_iter().collect(),
            node_outcomes: checkpoint.node_outcomes.clone().into_iter().collect(),
            next_node_id: checkpoint.next_node_id.clone(),
            git_commit_sha: checkpoint.git_commit_sha.clone(),
            loop_failure_signatures: std::collections::BTreeMap::new(),
            restart_failure_signatures: std::collections::BTreeMap::new(),
            node_visits: std::collections::BTreeMap::new(),
            diff: None,
            diff_summary: None,
        },
        workflow_event::Event::workflow_run_failed_from_error(
            &WorkflowError::Cancelled,
            fabro_types::RunTiming::wall_only(10),
            FailureReason::Cancelled,
            None,
            None,
            None,
            None,
        ),
    ])
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/start")))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "resume": true }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(run_json_status(&body)["kind"], "runnable");
    assert_eq!(body["timestamps"]["completed_at"], serde_json::Value::Null);
    assert_eq!(body["timing"], serde_json::Value::Null);

    let state = state
        .stores
        .runs
        .open_run_reader(&run_id)
        .await
        .unwrap()
        .state()
        .await
        .unwrap();
    assert_eq!(state.status, RunStatus::Runnable);
    assert!(state.conclusion.is_none());
}

#[tokio::test]
async fn retry_failed_run_creates_and_queues_new_run() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let source_run_id = RunId::new();
    create_durable_run_with_events(&state, source_run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::workflow_run_failed_from_error(
            &WorkflowError::engine("boom"),
            fabro_types::RunTiming::wall_only(10),
            FailureReason::WorkflowError,
            None,
            None,
            None,
            None,
        ),
    ])
    .await;
    let source_events_before = state
        .stores
        .runs
        .open_run(&source_run_id)
        .await
        .unwrap()
        .list_events()
        .await
        .unwrap()
        .len();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{source_run_id}/retry")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::CREATED).await;
    let new_run_id = body["id"].as_str().unwrap().parse::<RunId>().unwrap();

    assert_ne!(new_run_id, source_run_id);
    assert_eq!(body["retried_from"], source_run_id.to_string());
    assert_eq!(body["created_by"]["kind"], "user");
    assert_eq!(body["created_by"]["login"], "dev");
    assert_eq!(run_json_status(&body)["kind"], "runnable");

    let source_store = state.stores.runs.open_run(&source_run_id).await.unwrap();
    assert_eq!(
        source_store.list_events().await.unwrap().len(),
        source_events_before
    );
    assert_eq!(
        source_store.state().await.unwrap().status,
        RunStatus::Failed {
            reason: FailureReason::WorkflowError,
        }
    );

    let new_state = state
        .stores
        .runs
        .open_run(&new_run_id)
        .await
        .unwrap()
        .state()
        .await
        .unwrap();
    assert_eq!(new_state.retried_from, Some(source_run_id));
    assert_eq!(new_state.status, RunStatus::Runnable);
    assert!(new_state.checkpoints.is_empty());
}

#[tokio::test]
async fn retry_missing_run_returns_not_found() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{}/retry", fixtures::RUN_64)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn retry_succeeded_run_creates_and_queues_new_run() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let source_run_id = RunId::new();
    create_durable_run_with_events(&state, source_run_id, &[
        workflow_event::Event::WorkflowRunCompleted {
            timing:               fabro_types::RunTiming::wall_only(10),
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
    let source_events_before = state
        .stores
        .runs
        .open_run(&source_run_id)
        .await
        .unwrap()
        .list_events()
        .await
        .unwrap()
        .len();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{source_run_id}/retry")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::CREATED).await;
    let new_run_id = body["id"].as_str().unwrap().parse::<RunId>().unwrap();

    assert_ne!(new_run_id, source_run_id);
    assert_eq!(body["retried_from"], source_run_id.to_string());
    assert_eq!(run_json_status(&body)["kind"], "runnable");

    let source_store = state.stores.runs.open_run(&source_run_id).await.unwrap();
    assert_eq!(
        source_store.list_events().await.unwrap().len(),
        source_events_before
    );
    assert_eq!(
        source_store.state().await.unwrap().status,
        RunStatus::Succeeded {
            reason: SuccessReason::Completed,
        }
    );

    let new_state = state
        .stores
        .runs
        .open_run(&new_run_id)
        .await
        .unwrap()
        .state()
        .await
        .unwrap();
    assert_eq!(new_state.retried_from, Some(source_run_id));
    assert_eq!(new_state.status, RunStatus::Runnable);
}

#[tokio::test]
async fn retry_active_run_returns_conflict() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let source_run_id = RunId::new();
    create_durable_run_with_events(&state, source_run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
    ])
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{source_run_id}/retry")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_status!(response, StatusCode::CONFLICT).await;
}

#[tokio::test]
async fn cancel_run_succeeds() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id = create_and_start_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();

    // Cancel it
    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/cancel")))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    // Could be OK (cancelled) or CONFLICT (already completed)
    let status = response.status();
    assert!(
        status == StatusCode::OK || status == StatusCode::CONFLICT,
        "unexpected status: {status}"
    );
}

#[tokio::test]
async fn cancel_nonexistent_run_returns_not_found() {
    let app = test_app_with();
    let missing_run_id = fixtures::RUN_64;

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{missing_run_id}/cancel")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn cancel_terminal_durable_run_returns_conflict() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = fixtures::RUN_1;
    create_durable_run_with_events(&state, run_id, &[
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
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/cancel")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::CONFLICT).await;
    assert_eq!(
        body["errors"][0]["detail"],
        "Run is already terminal and cannot be cancelled."
    );
}

#[tokio::test]
async fn steer_interrupt_without_active_steerable_session_returns_conflict() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = fixtures::RUN_1;
    let (transport, _control_rx) = worker_transport_with_receiver(run_id).await;
    let _temp_dir = insert_running_control_run(&state, run_id, Some(transport));

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/steer")))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"text":"try again","interrupt":true}"#))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = body_json(response.into_body()).await;
    assert_eq!(body["errors"][0]["code"], "no_active_steerable_session");
}

#[tokio::test]
async fn interrupt_with_active_steerable_session_forwards_interrupt() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = fixtures::RUN_1;
    let stage_id = StageId::new("agent", 1);
    let (transport, mut control_rx) = worker_transport_with_receiver(run_id).await;
    let _temp_dir = insert_running_control_run(&state, run_id, Some(transport));
    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        runs.get_mut(&run_id)
            .unwrap()
            .active_steerable_stages
            .insert(stage_id, "session-a".to_string());
    }

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/interrupt")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::ACCEPTED).await;
    let envelope = recv_worker_control_envelope(&mut control_rx).await;
    assert!(matches!(
        envelope.message,
        WorkerControlMessage::Interrupt {
            actor: Principal::User(_),
        }
    ));
}

#[tokio::test]
async fn steer_interrupt_with_active_steerable_session_forwards_combined_control_message() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = fixtures::RUN_1;
    let stage_id = StageId::new("agent", 1);
    let (transport, mut control_rx) = worker_transport_with_receiver(run_id).await;
    let _temp_dir = insert_running_control_run(&state, run_id, Some(transport));
    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        runs.get_mut(&run_id)
            .unwrap()
            .active_steerable_stages
            .insert(stage_id, "session-a".to_string());
    }

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/steer")))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"text":"try again","interrupt":true}"#))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::ACCEPTED).await;
    let envelope = recv_worker_control_envelope(&mut control_rx).await;
    assert!(matches!(
        envelope.message,
        WorkerControlMessage::InterruptThenSteer { ref text, .. } if text == "try again"
    ));
}

#[tokio::test]
async fn interrupt_terminal_run_returns_run_not_interruptible() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = fixtures::RUN_1;
    let temp_dir = tempfile::tempdir().unwrap();
    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        runs.insert(
            run_id,
            managed_run(
                String::new(),
                RunStatus::Succeeded {
                    reason: SuccessReason::Completed,
                },
                chrono::Utc::now(),
                temp_dir.path().join(run_id.to_string()),
                RunExecutionMode::Start,
            ),
        );
    }

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/interrupt")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = body_json(response.into_body()).await;
    assert_eq!(body["errors"][0]["code"], "run_not_interruptible");
}

/// fabro-3fe4: the live status step is the shared lifecycle table plus two
/// documented deviations — the `RunRunnable` scheduling guard and the
/// shared `historic_skipped_statuses` replay path (the same fast-forward
/// the durable projection reducer uses). No AppState (or lock surgery) is
/// needed to prove it.

#[tokio::test]
async fn archive_and_unarchive_updates_listing_visibility() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = fixtures::RUN_1;

    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
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

    let archive_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/archive")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let archive_body = response_json!(archive_response, StatusCode::OK).await;
    assert!(run_json_archived(&archive_body));
    assert_eq!(run_json_status(&archive_body)["kind"], "succeeded");
    assert_eq!(run_json_status(&archive_body)["reason"], "completed");

    let hidden_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api("/runs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let hidden_body = response_json!(hidden_response, StatusCode::OK).await;
    assert!(
        !hidden_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| run_json_id(item) == Some(&run_id.to_string())),
        "archived run should be hidden from default listing"
    );

    let visible_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api("/runs?include_archived=true"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let visible_body = response_json!(visible_response, StatusCode::OK).await;
    let archived_item = visible_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| run_json_id(item) == Some(&run_id.to_string()))
        .expect("archived run should appear when include_archived=true");
    assert!(run_json_archived(archived_item));
    assert_eq!(run_json_status(archived_item)["kind"], "succeeded");
    assert_eq!(run_json_status(archived_item)["reason"], "completed");

    let unarchive_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/unarchive")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let unarchive_body = response_json!(unarchive_response, StatusCode::OK).await;
    assert!(!run_json_archived(&unarchive_body));
    assert_eq!(run_json_status(&unarchive_body)["kind"], "succeeded");
    assert_eq!(run_json_status(&unarchive_body)["reason"], "completed");

    let restored_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api("/runs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let restored_body = response_json!(restored_response, StatusCode::OK).await;
    let restored_item = restored_body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| run_json_id(item) == Some(&run_id.to_string()))
        .expect("unarchived run should reappear in default listing");
    assert_eq!(run_json_status(restored_item)["kind"], "succeeded");
    assert_eq!(run_json_status(restored_item)["reason"], "completed");
}

#[tokio::test]
async fn batch_archive_and_unarchive_updates_listing_visibility() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let first_id = RunId::new();
    let second_id = RunId::new();
    create_succeeded_run(&state, first_id).await;
    create_succeeded_run(&state, second_id).await;

    let archive_response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/runs/archive",
            &batch_lifecycle_body(&[first_id, second_id]),
        ))
        .await
        .unwrap();
    let archive_body = response_json!(archive_response, StatusCode::OK).await;
    assert_eq!(archive_body["summary"]["requested"], 2);
    assert_eq!(archive_body["summary"]["succeeded"], 2);
    assert_eq!(archive_body["summary"]["failed"], 0);
    let archive_results = archive_body["results"].as_array().unwrap();
    assert_batch_result(&archive_results[0], first_id, true, "archived");
    assert!(run_json_archived(&archive_results[0]["run"]));
    assert_batch_result(&archive_results[1], second_id, true, "archived");
    assert!(run_json_archived(&archive_results[1]["run"]));

    let hidden_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api("/runs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let hidden_body = response_json!(hidden_response, StatusCode::OK).await;
    assert!(
        hidden_body["data"].as_array().unwrap().iter().all(|item| {
            let item_id = run_json_id(item);
            item_id != Some(&first_id.to_string()) && item_id != Some(&second_id.to_string())
        }),
        "archived runs should be hidden from default listing"
    );

    let unarchive_response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/runs/unarchive",
            &batch_lifecycle_body(&[first_id, second_id]),
        ))
        .await
        .unwrap();
    let unarchive_body = response_json!(unarchive_response, StatusCode::OK).await;
    assert_eq!(unarchive_body["summary"]["requested"], 2);
    assert_eq!(unarchive_body["summary"]["succeeded"], 2);
    assert_eq!(unarchive_body["summary"]["failed"], 0);
    let unarchive_results = unarchive_body["results"].as_array().unwrap();
    assert_batch_result(&unarchive_results[0], first_id, true, "unarchived");
    assert!(!run_json_archived(&unarchive_results[0]["run"]));
    assert_batch_result(&unarchive_results[1], second_id, true, "unarchived");
    assert!(!run_json_archived(&unarchive_results[1]["run"]));

    let restored_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api("/runs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let restored_body = response_json!(restored_response, StatusCode::OK).await;
    for run_id in [first_id, second_id] {
        let restored_item = restored_body["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| run_json_id(item) == Some(&run_id.to_string()))
            .expect("unarchived run should reappear in default listing");
        assert_eq!(run_json_status(restored_item)["kind"], "succeeded");
    }
}

#[tokio::test]
async fn batch_archive_reports_ordered_mixed_results_without_rollback() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let already_archived_id = RunId::new();
    let terminal_id = RunId::new();
    let running_id = RunId::new();
    let missing_id = RunId::new();
    create_succeeded_run(&state, already_archived_id).await;
    create_succeeded_run(&state, terminal_id).await;
    create_running_run(&state, running_id).await;

    let already_archived_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{already_archived_id}/archive")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(already_archived_response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/runs/archive",
            &batch_lifecycle_body(&[already_archived_id, terminal_id, running_id, missing_id]),
        ))
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["summary"]["requested"], 4);
    assert_eq!(body["summary"]["succeeded"], 2);
    assert_eq!(body["summary"]["failed"], 2);
    let results = body["results"].as_array().unwrap();
    assert_batch_result(&results[0], already_archived_id, true, "already_archived");
    assert_batch_result(&results[1], terminal_id, true, "archived");
    assert_batch_result(&results[2], running_id, false, "conflict");
    assert_eq!(results[2]["error"]["status"], "409");
    assert_batch_result(&results[3], missing_id, false, "not_found");
    assert_eq!(results[3]["error"]["status"], "404");

    let terminal_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{terminal_id}")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let terminal_body = response_json!(terminal_response, StatusCode::OK).await;
    assert!(run_json_archived(&terminal_body));
}

#[tokio::test]
async fn batch_unarchive_treats_terminal_not_archived_as_success() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let archived_id = RunId::new();
    let not_archived_id = RunId::new();
    create_succeeded_run(&state, archived_id).await;
    create_succeeded_run(&state, not_archived_id).await;

    let archive_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{archived_id}/archive")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(archive_response, StatusCode::OK).await;

    let response = app
        .oneshot(json_request(
            Method::POST,
            "/runs/unarchive",
            &batch_lifecycle_body(&[archived_id, not_archived_id]),
        ))
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["summary"]["requested"], 2);
    assert_eq!(body["summary"]["succeeded"], 2);
    assert_eq!(body["summary"]["failed"], 0);
    let results = body["results"].as_array().unwrap();
    assert_batch_result(&results[0], archived_id, true, "unarchived");
    assert_batch_result(&results[1], not_archived_id, true, "not_archived");
}

#[tokio::test]
async fn batch_lifecycle_rejects_invalid_requests_before_mutating_runs() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_succeeded_run(&state, run_id).await;
    let too_many_ids = (0..251)
        .map(|_| RunId::new().to_string())
        .collect::<Vec<_>>();
    let invalid_requests = [
        json!({ "run_ids": [] }),
        json!({ "run_ids": [run_id.to_string(), run_id.to_string()] }),
        json!({ "run_ids": ["not-a-run-id"] }),
        json!({ "run_ids": too_many_ids }),
    ];

    for body in invalid_requests {
        let response = app
            .clone()
            .oneshot(json_request(Method::POST, "/runs/archive", &body))
            .await
            .unwrap();
        assert_status!(response, StatusCode::BAD_REQUEST).await;
    }

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert!(!run_json_archived(&body));
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
async fn archive_unknown_run_returns_not_found() {
    let app = test_app_with();
    let run_id = fixtures::RUN_64;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/archive")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_status!(response, StatusCode::NOT_FOUND).await;
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

#[expect(
    clippy::disallowed_methods,
    reason = "test asserts the raw template source"
)]
#[tokio::test]
async fn start_run_persists_full_settings_snapshot() {
    let source = r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[run.execution]
mode = "dry_run"

[run.model]
provider = "anthropic"
name = "claude-sonnet-4-5"

[run.environment]
id = "local"

[[run.hooks]]
name = "snapshot-hook"
event = "run_start"
command = ["echo", "snapshot"]
blocking = false
timeout = "1s"
sandbox = false

[run.git.author]
name = "Snapshot Bot"
email = "snapshot@example.com"

[server.integrations.github]
app_id = "12345"

[server.web]
url = "http://example.test"

[server.api]
url = "http://api.example.test"

[server.logging]
level = "debug"
"#;
    let state = test_app_state_with_options(
        server_settings_from_toml(source),
        manifest_run_defaults_from_toml(source),
        5,
    );
    state
        .stores
        .vault
        .set(
            EnvVars::ANTHROPIC_API_KEY,
            "test-anthropic-api-key",
            SecretType::Token,
            None,
        )
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let req = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(intent_body(&app, MINIMAL_DOT).await)
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::CREATED).await;
    let run_id = body["id"].as_str().unwrap().parse::<RunId>().unwrap();

    let _run_dir = {
        let runs = state.runs.lock().expect("runs lock poisoned");
        runs.get(&run_id)
            .and_then(|run| run.run_dir.clone())
            .expect("run_dir should be recorded")
    };
    let run_spec = state
        .stores
        .runs
        .open_run_reader(&run_id)
        .await
        .unwrap()
        .state()
        .await
        .unwrap()
        .spec;
    let resolved_run = &run_spec.settings.run;

    // Verify a sampling of the persisted v2 settings, including inherited
    // run execution mode from server settings.
    assert_eq!(
        match &resolved_run.goal {
            Some(fabro_types::settings::run::RunGoal::Inline(value)) => Some(value.as_source()),
            _ => None,
        }
        .as_deref(),
        Some("Test"),
        "goal should be persisted from the manifest"
    );
    assert!(
        resolved_run.execution.mode == RunMode::DryRun,
        "run execution mode should inherit from server settings"
    );
    assert_eq!(
        resolved_run.model.name.as_deref(),
        Some("claude-sonnet-4.5"),
    );

    // Server-operational fields (auth, integrations, etc.) deliberately
    // do not flow into the run's persisted settings — they live on the
    // server and are read via AppState::server_settings().
    let settings_json = serde_json::to_value(&run_spec.settings).unwrap();
    assert!(settings_json.pointer("/server").is_none());
}

#[tokio::test]
async fn cancel_runnable_run_succeeds() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id = create_and_start_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();

    // Cancel it
    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/cancel")))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::OK).await;

    // Verify status is cancelled
    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}")))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    assert_eq!(run_json_status(&body)["kind"], "failed");
    assert_eq!(run_json_status(&body)["reason"], "cancelled");

    // Cancelled runs appear in the runs list with a "failed" status
    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    let run_id_str = run_id.to_string();
    let list_item = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| run_json_id(item) == Some(run_id_str.as_str()));
    assert!(
        list_item.is_some(),
        "cancelled run should appear in the list"
    );
    assert_eq!(
        run_json_status(list_item.unwrap())["kind"].as_str(),
        Some("failed"),
        "cancelled run should preserve the failed lifecycle status"
    );

    let run_store = state.stores.runs.open_run_reader(&run_id).await.unwrap();
    let status = run_store.state().await.unwrap().status;
    assert_eq!(status, RunStatus::Failed {
        reason: FailureReason::Cancelled,
    });
}

#[tokio::test]
async fn cancel_run_overwrites_pending_pause_request() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id_str = create_and_start_run(&app, MINIMAL_DOT).await;
    let run_id = run_id_str.parse::<RunId>().unwrap();

    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let managed_run = runs.get_mut(&run_id).expect("run should exist");
        managed_run.status = RunStatus::Running;
        managed_run.worker_ref = Some(test_worker_ref(u32::MAX));
    }
    append_control_request(state.as_ref(), run_id, RunControlAction::Pause, None)
        .await
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/cancel")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::ACCEPTED).await;
    assert_eq!(run_json_pending_control(&body).as_str(), Some("cancel"));

    let summary = state
        .stores
        .run_summaries
        .get(&run_id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        summary.lifecycle.pending_control,
        Some(RunControlAction::Cancel)
    );
}

#[tokio::test]
async fn cancel_run_requests_worker_runtime_stop_when_control_unavailable() {
    let runtime = StdArc::new(RecordingWorkerRuntime::default());
    runtime.set_alive(true);
    let state = TestAppStateBuilder::new()
        .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
        .worker_runtime(runtime.clone())
        .build();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();
    let worker_ref = test_worker_ref(u32::MAX);

    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let managed_run = runs.get_mut(&run_id).expect("run should exist");
        managed_run.status = RunStatus::Running;
        managed_run.answer_transport = None;
        managed_run.worker_ref = Some(worker_ref.clone());
    }

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/cancel")))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::ACCEPTED).await;

    assert_eq!(runtime.requested_refs(), vec![worker_ref.clone()]);

    tokio::time::pause();
    advance_past_worker_cancel_grace().await;
    runtime.wait_for_forced_ref(&worker_ref).await;

    assert_eq!(runtime.forced_refs(), vec![worker_ref]);
}

#[tokio::test]
async fn cancel_run_force_stops_worker_when_delivered_control_does_not_converge() {
    let runtime = StdArc::new(RecordingWorkerRuntime::default());
    runtime.set_alive(true);
    let state = TestAppStateBuilder::new()
        .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
        .worker_runtime(runtime.clone())
        .build();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();
    let worker_ref = test_worker_ref(u32::MAX);
    let (answer_transport, _receiver) = worker_transport_with_receiver(run_id).await;

    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let managed_run = runs.get_mut(&run_id).expect("run should exist");
        managed_run.status = RunStatus::Running;
        managed_run.answer_transport = Some(answer_transport);
        managed_run.worker_ref = Some(worker_ref.clone());
    }

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/cancel")))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::ACCEPTED).await;

    assert!(runtime.requested_refs().is_empty());
    assert!(runtime.forced_refs().is_empty());

    tokio::time::pause();
    advance_past_worker_cancel_grace().await;
    runtime.wait_for_forced_ref(&worker_ref).await;

    assert_eq!(runtime.forced_refs(), vec![worker_ref]);
}

#[tokio::test]
async fn cancel_run_watchdog_does_not_stop_replacement_worker() {
    let runtime = StdArc::new(RecordingWorkerRuntime::default());
    runtime.set_alive(true);
    let state = TestAppStateBuilder::new()
        .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
        .worker_runtime(runtime.clone())
        .build();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();
    let cancelled_worker_ref = test_worker_ref(u32::MAX - 1);
    let replacement_worker_ref = test_worker_ref(u32::MAX);
    let (answer_transport, _receiver) = worker_transport_with_receiver(run_id).await;

    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let managed_run = runs.get_mut(&run_id).expect("run should exist");
        managed_run.status = RunStatus::Running;
        managed_run.answer_transport = Some(answer_transport);
        managed_run.worker_ref = Some(cancelled_worker_ref);
    }

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/cancel")))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::ACCEPTED).await;

    tokio::time::pause();
    tokio::task::yield_now().await;
    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let managed_run = runs.get_mut(&run_id).expect("run should exist");
        managed_run.worker_ref = Some(replacement_worker_ref);
    }
    advance_past_worker_cancel_grace().await;

    assert!(runtime.forced_refs().is_empty());
}

#[tokio::test]
async fn cancel_run_watchdog_does_not_stop_worker_after_live_ref_clears() {
    let runtime = StdArc::new(RecordingWorkerRuntime::default());
    runtime.set_alive(true);
    let state = TestAppStateBuilder::new()
        .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
        .worker_runtime(runtime.clone())
        .build();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();
    let worker_ref = test_worker_ref(u32::MAX);
    let (answer_transport, _receiver) = worker_transport_with_receiver(run_id).await;

    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let managed_run = runs.get_mut(&run_id).expect("run should exist");
        managed_run.status = RunStatus::Running;
        managed_run.answer_transport = Some(answer_transport);
        managed_run.worker_ref = Some(worker_ref);
    }

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/cancel")))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::ACCEPTED).await;

    tokio::time::pause();
    tokio::task::yield_now().await;
    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let managed_run = runs.get_mut(&run_id).expect("run should exist");
        managed_run.worker_ref = None;
    }
    advance_past_worker_cancel_grace().await;

    assert!(runtime.forced_refs().is_empty());
}

#[tokio::test]
async fn repeated_cancel_request_arms_one_watchdog_and_persists_one_intent() {
    let runtime = StdArc::new(RecordingWorkerRuntime::default());
    runtime.set_alive(true);
    let state = TestAppStateBuilder::new()
        .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
        .worker_runtime(runtime.clone())
        .build();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();
    let worker_ref = test_worker_ref(u32::MAX);
    let (answer_transport, _receiver) = worker_transport_with_receiver(run_id).await;

    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let managed_run = runs.get_mut(&run_id).expect("run should exist");
        managed_run.status = RunStatus::Running;
        managed_run.answer_transport = Some(answer_transport);
        managed_run.worker_ref = Some(worker_ref.clone());
    }

    let first_request = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/cancel")))
        .body(Body::empty())
        .unwrap();
    let second_request = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/cancel")))
        .body(Body::empty())
        .unwrap();
    let (first_response, second_response) = tokio::join!(
        app.clone().oneshot(first_request),
        app.clone().oneshot(second_request)
    );
    assert_status!(first_response.unwrap(), StatusCode::ACCEPTED).await;
    assert_status!(second_response.unwrap(), StatusCode::ACCEPTED).await;

    let run_store = state.stores.runs.open_run_reader(&run_id).await.unwrap();
    let request_count = run_store
        .list_events()
        .await
        .unwrap()
        .iter()
        .filter(|event| event.event.event_name() == "run.cancel.requested")
        .count();
    assert_eq!(request_count, 1);

    tokio::time::pause();
    advance_past_worker_cancel_grace().await;
    runtime.wait_for_forced_ref(&worker_ref).await;

    assert_eq!(runtime.forced_refs(), vec![worker_ref]);
}

#[tokio::test]
async fn cancel_durably_blocked_in_process_run_cancels_pending_interview_without_abort_signal() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = fixtures::RUN_1;
    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunRunning,
        workflow_event::Event::RunBlocked {
            blocked_reason: BlockedReason::HumanInputRequired,
        },
    ])
    .await;

    let interviewer = Arc::new(ControlInterviewer::new());
    let mut question = Question::new("approve?", QuestionType::YesNo);
    question.id = "q-1".to_string();
    let ask_interviewer = Arc::clone(&interviewer);
    let ask = tokio::spawn(async move { ask_interviewer.ask(question).await });
    tokio::task::yield_now().await;

    let (cancel_tx, mut cancel_rx) = oneshot::channel();
    let cancel_token = CancellationToken::new();
    let temp_dir = tempfile::tempdir().unwrap();
    let mut run = managed_run(
        MINIMAL_DOT.to_string(),
        RunStatus::Running,
        Utc::now(),
        temp_dir.path().join(run_id.to_string()),
        RunExecutionMode::Start,
    );
    run.answer_transport = Some(RunAnswerTransport::InProcess {
        interviewer,
        steering_hub: Arc::new(fabro_workflow::SteeringHub::new(Arc::new(
            fabro_workflow::event::Emitter::new(run_id),
        ))),
    });
    run.cancel_token = Some(cancel_token);
    run.cancel_tx = Some(cancel_tx);
    state
        .runs
        .lock()
        .expect("runs lock poisoned")
        .insert(run_id, run);

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/cancel")))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::ACCEPTED).await;

    let submission = tokio::time::timeout(std::time::Duration::from_millis(100), ask)
        .await
        .expect("cancel should resolve the pending in-process interview")
        .expect("interview task should not panic");
    assert_eq!(submission.answer.value, AnswerValue::Cancelled);
    assert!(
        matches!(
            cancel_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ),
        "blocked in-process cancellation should let the workflow unwind instead of aborting it"
    );
}

#[tokio::test]
async fn pause_run_rejects_when_control_is_already_pending() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id_str = create_and_start_run(&app, MINIMAL_DOT).await;
    let run_id = run_id_str.parse::<RunId>().unwrap();

    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let managed_run = runs.get_mut(&run_id).expect("run should exist");
        managed_run.status = RunStatus::Running;
        managed_run.worker_ref = Some(test_worker_ref(u32::MAX));
    }
    append_control_request(state.as_ref(), run_id, RunControlAction::Cancel, None)
        .await
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/pause")))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::CONFLICT).await;

    let summary = state
        .stores
        .run_summaries
        .get(&run_id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        summary.lifecycle.pending_control,
        Some(RunControlAction::Cancel)
    );
}

#[tokio::test]
async fn pause_run_sets_pending_control_on_board_response() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id_str = create_and_start_run(&app, MINIMAL_DOT).await;
    let run_id = run_id_str.parse::<RunId>().unwrap();

    let (transport, _control_rx) = worker_transport_with_receiver(run_id).await;
    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let managed_run = runs.get_mut(&run_id).expect("run should exist");
        managed_run.status = RunStatus::Running;
        managed_run.worker_ref = Some(test_worker_ref(u32::MAX));
        managed_run.answer_transport = Some(transport);
    }

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/pause")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(run_json_status(&body)["kind"], "runnable");
    assert_eq!(run_json_pending_control(&body).as_str(), Some("pause"));

    // Verify pending_control via /runs/{id} (board no longer includes this field)
    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    assert_eq!(run_json_pending_control(&body).as_str(), Some("pause"));

    // Verify the run appears in the runs list with runnable status.
    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    let item = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| run_json_id(item) == Some(run_id_str.as_str()))
        .expect("board item should exist");
    assert!(run_json_status(item).is_object());
    assert_eq!(run_json_pending_control(item).as_str(), Some("pause"));
}

#[tokio::test]
async fn pause_run_immediately_pauses_blocked_run() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id_str = create_and_start_run(&app, MINIMAL_DOT).await;
    let run_id = run_id_str.parse::<RunId>().unwrap();

    append_raw_run_event(
        &state,
        run_id,
        "pause-starting",
        "2026-04-19T11:59:58Z",
        "run.starting",
        json!({}),
        None,
    )
    .await;
    append_raw_run_event(
        &state,
        run_id,
        "pause-running",
        "2026-04-19T11:59:59Z",
        "run.running",
        json!({}),
        None,
    )
    .await;
    append_raw_run_event(
        &state,
        run_id,
        "pause-blocked",
        "2026-04-19T12:00:00Z",
        "run.blocked",
        json!({ "blocked_reason": "human_input_required" }),
        None,
    )
    .await;

    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let managed_run = runs.get_mut(&run_id).expect("run should exist");
        managed_run.status = RunStatus::Blocked {
            blocked_reason: BlockedReason::HumanInputRequired,
        };
        managed_run.worker_ref = Some(test_worker_ref(u32::MAX));
    }

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/pause")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(run_json_status(&body)["kind"], "paused");
    assert_eq!(
        run_json_status(&body)["prior_block"],
        "human_input_required"
    );
    assert_eq!(run_json_pending_control(&body), &serde_json::Value::Null);

    let summary = state
        .stores
        .run_summaries
        .get(&run_id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.lifecycle.status, RunStatus::Paused {
        prior_block: Some(BlockedReason::HumanInputRequired),
    });
    assert_eq!(summary.lifecycle.pending_control, None);
}

#[tokio::test]
async fn unpause_run_sets_pending_control() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id_str = create_and_start_run(&app, MINIMAL_DOT).await;
    let run_id = run_id_str.parse::<RunId>().unwrap();

    let (transport, _control_rx) = worker_transport_with_receiver(run_id).await;
    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let managed_run = runs.get_mut(&run_id).expect("run should exist");
        managed_run.status = RunStatus::Paused { prior_block: None };
        managed_run.worker_ref = Some(test_worker_ref(u32::MAX));
        managed_run.answer_transport = Some(transport);
    }

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/unpause")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(run_json_status(&body)["kind"], "runnable");
    assert_eq!(run_json_pending_control(&body).as_str(), Some("unpause"));

    let summary = state
        .stores
        .run_summaries
        .get(&run_id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        summary.lifecycle.pending_control,
        Some(RunControlAction::Unpause)
    );
}

#[tokio::test]
async fn unpause_run_returns_blocked_when_human_gate_is_still_unresolved() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id_str = create_and_start_run(&app, MINIMAL_DOT).await;
    let run_id = run_id_str.parse::<RunId>().unwrap();

    append_raw_run_event(
        &state,
        run_id,
        "paused-blocked-starting",
        "2026-04-19T11:59:58Z",
        "run.starting",
        json!({}),
        None,
    )
    .await;
    append_raw_run_event(
        &state,
        run_id,
        "paused-blocked-running",
        "2026-04-19T11:59:59Z",
        "run.running",
        json!({}),
        None,
    )
    .await;
    append_raw_run_event(
        &state,
        run_id,
        "paused-blocked-paused",
        "2026-04-19T12:00:00Z",
        "run.paused",
        json!({}),
        None,
    )
    .await;
    append_raw_run_event(
        &state,
        run_id,
        "paused-blocked-status",
        "2026-04-19T12:00:01Z",
        "run.blocked",
        json!({ "blocked_reason": "human_input_required" }),
        None,
    )
    .await;

    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let managed_run = runs.get_mut(&run_id).expect("run should exist");
        managed_run.status = RunStatus::Paused {
            prior_block: Some(BlockedReason::HumanInputRequired),
        };
        managed_run.worker_ref = Some(test_worker_ref(u32::MAX));
    }

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/unpause")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(run_json_status(&body)["kind"], "blocked");
    assert_eq!(
        run_json_status(&body)["blocked_reason"],
        "human_input_required"
    );
    assert_eq!(run_json_pending_control(&body), &serde_json::Value::Null);

    let summary = state
        .stores
        .run_summaries
        .get(&run_id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.lifecycle.status, RunStatus::Blocked {
        blocked_reason: BlockedReason::HumanInputRequired,
    });
    assert_eq!(summary.lifecycle.pending_control, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_during_startup_persists_cancelled_reason() {
    let source = r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[[run.prepare.steps]]
script = "sleep 5"

[run.prepare]
timeout = "30s"

[run.environment]
id = "local"
"#;
    let state = test_app_state_with_settings_and_registry_factory(
        server_settings_from_toml(source),
        manifest_run_defaults_from_toml(source),
        |interviewer| fabro_workflow::handler::default_registry(interviewer, || None),
    );
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let workspace = tempfile::tempdir().unwrap();
    let mut intent = test_intent(&app, MINIMAL_DOT).await;
    intent["target"] = json!({"kind": "folder", "path": workspace.path()});
    intent["environment_id"] = json!("local");
    let run_id_str = post_run_intent(&app, intent).await["id"]
        .as_str()
        .unwrap()
        .to_string();
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id_str}/start")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let run_id = run_id_str.parse::<RunId>().unwrap();

    let runner = tokio::spawn(
        execute_run(Arc::clone(&state), run_id)
            .instrument(tracing::info_span!("run", id = %run_id)),
    );
    let mut live_status_before_cancel = None;
    for _ in 0..50 {
        live_status_before_cancel = {
            let runs = state.runs.lock().expect("runs lock poisoned");
            runs.get(&run_id).map(|run| run.status)
        };
        if matches!(
            live_status_before_cancel,
            Some(
                RunStatus::Starting
                    | RunStatus::Running
                    | RunStatus::Blocked { .. }
                    | RunStatus::Paused { .. }
            )
        ) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        matches!(
            live_status_before_cancel,
            Some(
                RunStatus::Starting
                    | RunStatus::Running
                    | RunStatus::Blocked { .. }
                    | RunStatus::Paused { .. }
            )
        ),
        "run should become cancellable before finishing, saw {live_status_before_cancel:?}"
    );

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/cancel")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let response_status = response.status();
    let response_body = body_json(response.into_body()).await;
    assert_eq!(
        response_status,
        StatusCode::ACCEPTED,
        "unexpected cancel response body: {response_body}; live status before cancel: {live_status_before_cancel:?}"
    );

    runner.await.unwrap();

    let runs = state.runs.lock().expect("runs lock poisoned");
    let managed_run = runs.get(&run_id).expect("run should exist");
    assert_eq!(managed_run.status, RunStatus::Failed {
        reason: FailureReason::Cancelled,
    });
    drop(runs);

    let run_store = state.stores.runs.open_run_reader(&run_id).await.unwrap();

    let mut status_record = None;
    for _ in 0..50 {
        let record = run_store.state().await.unwrap().status;
        if record
            == (RunStatus::Failed {
                reason: FailureReason::Cancelled,
            })
        {
            status_record = Some(record);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    let status_record = status_record.expect("status record should be persisted");
    assert_eq!(status_record, RunStatus::Failed {
        reason: FailureReason::Cancelled,
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[expect(
    clippy::disallowed_methods,
    reason = "This test intentionally blocks inside a sync registry factory to simulate slow startup before cancellation."
)]
async fn cancel_before_run_transitions_to_running_returns_empty_attach_stream() {
    let state = test_app_state_with_registry_factory(|interviewer| {
        std::thread::sleep(std::time::Duration::from_millis(200));
        fabro_workflow::handler::default_registry(interviewer, || None)
    });
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id_str = create_and_start_run(&app, MINIMAL_DOT).await;
    let run_id = run_id_str.parse::<RunId>().unwrap();

    let runner = tokio::spawn(
        execute_run(Arc::clone(&state), run_id)
            .instrument(tracing::info_span!("run", id = %run_id)),
    );
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/cancel")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::ACCEPTED).await;

    runner.await.unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/attach")))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_bytes!(response, StatusCode::OK).await;
    assert!(
        body.is_empty(),
        "expected an empty attach stream, got {}",
        String::from_utf8_lossy(&body)
    );
}

/// Reasoning has to survive the whole durable path, not just the local
/// struct conversion: emitted event → run store → attach SSE JSON.

#[tokio::test]
async fn submit_answer_to_unstarted_run_returns_conflict() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(state);

    let req = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(intent_body(&app, MINIMAL_DOT).await)
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    let run_id = body["id"].as_str().unwrap().to_string();

    // Try to submit an answer to a run with no active worker.
    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/questions/q1/answer")))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({"kind": "yes"})).unwrap(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::CONFLICT).await;
}
