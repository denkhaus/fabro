use super::*;

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
async fn persist_cancelled_run_status_ignores_already_terminal_runs() {
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

    persist_cancelled_run_status(state.as_ref(), run_id)
        .await
        .unwrap();

    let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
    let projection = run_store.state().await.unwrap();
    assert_eq!(projection.status, RunStatus::Succeeded {
        reason: SuccessReason::Completed,
    });
    assert!(!run_store.list_events().await.unwrap().iter().any(|event| {
        matches!(
            event.event.body,
            EventBody::RunFailed(ref props) if props.failure.reason == FailureReason::Cancelled
        )
    }));
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

#[tokio::test]
async fn list_run_stages_projects_running_stage_as_cancelled_after_cancelled_run_failure() {
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
            index:                 1,
            handler_type:          "agent".to_string(),
            attempt:               1,
            max_attempts:          1,
        },
    )
    .await;
    let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
    workflow_event::append_event(
        &run_store,
        &run_id,
        &workflow_event::Event::WorkflowRunFailed {
            failure:              fabro_types::RunFailure {
                reason: fabro_types::FailureReason::Cancelled,
                detail: FailureDetail::new("cancelled", FailureCategory::Canceled),
            },
            timing:               fabro_types::RunTiming::wall_only(100),
            final_git_commit_sha: None,
            final_patch:          None,
            diff_summary:         None,
            usage:                None,
        },
    )
    .await
    .unwrap();

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
    assert_eq!(stage_status(&body, "work@1"), "cancelled");
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
async fn steer_nonexistent_run_returns_not_found() {
    let app = test_app_with();
    let missing_run_id = fixtures::RUN_64;

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{missing_run_id}/steer")))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"text":"try again"}"#))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn steer_terminal_durable_run_returns_run_not_steerable() {
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
        .uri(api(&format!("/runs/{run_id}/steer")))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"text":"try again"}"#))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::CONFLICT).await;
    assert_eq!(body["errors"][0]["code"], "run_not_steerable");
    assert_eq!(body["errors"][0]["detail"], "Run is no longer steerable.");
}

#[tokio::test]
async fn steer_empty_text_returns_bad_request() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id = create_and_start_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/steer")))
        .header("content-type", "application/json")
        .body(Body::from(r#"{"text":"   "}"#))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    // 400 (whitespace-only text) or 409 (run not yet `running` when the
    // handler checks status) are both acceptable; the only outcome we
    // want to rule out is a successful enqueue.
    let status = response.status();
    assert!(
        matches!(status, StatusCode::BAD_REQUEST | StatusCode::CONFLICT),
        "expected 400 or 409, got {status}"
    );
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
#[test]
fn live_status_step_matches_the_shared_table_except_runnable_suppression() {
    use fabro_types::{LifecycleTransition, RunRunnableSource, apply_lifecycle_event};

    let run_id = fixtures::RUN_1;
    let runnable = workflow_event::to_run_event(&run_id, &workflow_event::Event::RunRunnable {
        source: RunRunnableSource::StartRequested,
        actor:  None,
    });
    // Live-only scheduling guard: an injected runnable event never flips
    // status, no matter the current status.
    assert_eq!(
        super::live_lifecycle_status(RunStatus::Submitted, &runnable),
        Ok(None)
    );
    assert_eq!(
        super::live_lifecycle_status(RunStatus::Runnable, &runnable),
        Ok(None)
    );

    let lifecycle_events = vec![
        workflow_event::to_run_event(&run_id, &workflow_event::Event::RunStarting),
        workflow_event::to_run_event(&run_id, &workflow_event::Event::RunRunning),
        workflow_event::to_run_event(&run_id, &workflow_event::Event::WorkflowRunCompleted {
            timing:               fabro_types::RunTiming::wall_only(1),
            artifact_count:       0,
            status:               "succeeded".to_string(),
            reason:               SuccessReason::Completed,
            failure:              None,
            final_git_commit_sha: None,
            final_patch:          None,
            diff_summary:         None,
            usage:                None,
        }),
        workflow_event::to_run_event(&run_id, &workflow_event::Event::WorkflowRunFailed {
            failure:              fabro_types::RunFailure {
                reason: fabro_types::FailureReason::Cancelled,
                detail: FailureDetail::new(
                    "cancelled before start",
                    FailureCategory::Deterministic,
                ),
            },
            timing:               fabro_types::RunTiming::wall_only(1),
            final_git_commit_sha: None,
            final_patch:          None,
            diff_summary:         None,
            usage:                None,
        }),
    ];
    let statuses = [
        RunStatus::Submitted,
        RunStatus::Runnable,
        RunStatus::Starting,
        RunStatus::Running,
        RunStatus::Paused { prior_block: None },
        RunStatus::Failed {
            reason: fabro_types::FailureReason::Cancelled,
        },
    ];

    for status in statuses {
        for event in &lifecycle_events {
            // The shared historic fast-forward replays the same intermediate
            // statuses the durable reducer replays, so the live fold agrees
            // with a full-table fold from the fast-forwarded status.
            let mut table_status = status;
            for skipped in fabro_types::historic_skipped_statuses(table_status, &event.body) {
                table_status = skipped;
            }
            let expected = match apply_lifecycle_event(table_status, &event.body) {
                LifecycleTransition::Next(next) => Ok(Some(next)),
                LifecycleTransition::Rejected(err) => Err(err),
                LifecycleTransition::NotLifecycle => Ok(None),
            };
            assert_eq!(
                super::live_lifecycle_status(status, event),
                expected,
                "live step drifted from the table at {status}"
            );
        }
    }
}

#[test]
fn injected_runnable_event_does_not_make_submitted_run_schedulable() {
    let state = test_app_state();
    let run_id = fixtures::RUN_1;
    let temp_dir = tempfile::tempdir().unwrap();
    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        runs.insert(
            run_id,
            managed_run(
                String::new(),
                RunStatus::Submitted,
                chrono::Utc::now(),
                temp_dir.path().join(run_id.to_string()),
                RunExecutionMode::Start,
            ),
        );
    }

    let runnable = workflow_event::to_run_event(&run_id, &workflow_event::Event::RunRunnable {
        source: fabro_types::RunRunnableSource::StartRequested,
        actor:  None,
    });
    update_live_run_from_event(&state, run_id, &runnable);

    {
        let runs = state.runs.lock().expect("runs lock poisoned");
        assert_eq!(runs.get(&run_id).unwrap().status, RunStatus::Submitted);
    }

    let starting = workflow_event::to_run_event(&run_id, &workflow_event::Event::RunStarting);
    update_live_run_from_event(&state, run_id, &starting);

    let runs = state.runs.lock().expect("runs lock poisoned");
    // The runnable guard keeps scheduling ownership with the lifecycle
    // handlers, but a follow-up `run.starting` still advances the live
    // status through the shared historic fast-forward (submitted ->
    // runnable -> starting) — the same replay the durable projection
    // reducer performs, so live and durable folds stay equivalent and
    // seeded event sequences reach terminal statuses (fabro-3fe4 reopen:
    // the strict rejection stranded injected runs at `submitted` and broke
    // `fabro rm` completed-run handling).
    assert_eq!(runs.get(&run_id).unwrap().status, RunStatus::Starting);
}

#[test]
fn active_steerable_stage_projection_ignores_stale_deactivation() {
    let state = test_app_state();
    let run_id = fixtures::RUN_1;
    let temp_dir = tempfile::tempdir().unwrap();
    {
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        runs.insert(
            run_id,
            managed_run(
                String::new(),
                RunStatus::Running,
                chrono::Utc::now(),
                temp_dir.path().join(run_id.to_string()),
                RunExecutionMode::Start,
            ),
        );
    }

    let stage_id = StageId::new("agent", 1);
    let activated_a =
        workflow_event::to_run_event(&run_id, &workflow_event::Event::AgentSessionActivated {
            node_id:          "agent".to_string(),
            visit:            1,
            session_id:       "session-a".to_string(),
            thread_id:        None,
            provider:         Some("openai".to_string()),
            model:            Some("gpt-5.4".to_string()),
            reasoning_effort: None,
            speed:            None,
            permission_level: None,
            capabilities:     vec![SessionCapability::Steer],
        });
    update_live_run_from_event(&state, run_id, &activated_a);

    let deactivated_a =
        workflow_event::to_run_event(&run_id, &workflow_event::Event::AgentSessionDeactivated {
            node_id:    "agent".to_string(),
            visit:      1,
            session_id: "session-a".to_string(),
        });
    update_live_run_from_event(&state, run_id, &deactivated_a);

    let activated_b =
        workflow_event::to_run_event(&run_id, &workflow_event::Event::AgentSessionActivated {
            node_id:          "agent".to_string(),
            visit:            1,
            session_id:       "session-b".to_string(),
            thread_id:        None,
            provider:         Some("openai".to_string()),
            model:            Some("gpt-5.4".to_string()),
            reasoning_effort: None,
            speed:            None,
            permission_level: None,
            capabilities:     vec![SessionCapability::Steer],
        });
    update_live_run_from_event(&state, run_id, &activated_b);
    update_live_run_from_event(&state, run_id, &deactivated_a);

    let runs = state.runs.lock().expect("runs lock poisoned");
    let run = runs.get(&run_id).unwrap();
    assert_eq!(
        run.active_steerable_stages
            .get(&stage_id)
            .map(String::as_str),
        Some("session-b")
    );
}

#[tokio::test]
async fn active_acp_steerable_marker_clears_on_terminal_paths() {
    let terminal_events: Vec<workflow_event::Event> = vec![
        workflow_event::Event::AgentAcpCompleted {
            node_id:     "agent".to_string(),
            stdout:      "done".to_string(),
            stderr:      String::new(),
            stop_reason: "end_turn".to_string(),
            duration_ms: 42,
        },
        workflow_event::Event::AgentAcpCancelled {
            node_id:     "agent".to_string(),
            stdout:      "partial".to_string(),
            stderr:      "cancelled".to_string(),
            duration_ms: 7,
        },
        workflow_event::Event::AgentAcpTimedOut {
            node_id:     "agent".to_string(),
            stdout:      "partial".to_string(),
            stderr:      "timeout".to_string(),
            duration_ms: 99,
        },
        workflow_event::Event::StageCompleted {
            node_id: "agent".to_string(),
            name: "agent".to_string(),
            index: 0,
            timing: fabro_types::StageTiming::wall_only(1),
            status: "success".to_string(),
            preferred_label: None,
            suggested_next_ids: Vec::new(),
            usage_by_model: Vec::new(),
            usage: None,
            failure: None,
            notes: None,
            files_touched: Vec::new(),
            context_updates: None,
            jump_to_node: None,
            context_values: None,
            node_visits: None,
            loop_failure_signatures: None,
            restart_failure_signatures: None,
            response: None,
            attempt: 1,
            max_attempts: 1,
        },
        workflow_event::Event::StageFailed {
            node_id:        "agent".to_string(),
            name:           "agent".to_string(),
            index:          0,
            failure:        FailureDetail::new("failed", FailureCategory::Deterministic),
            will_retry:     false,
            timing:         fabro_types::StageTiming::wall_only(1),
            usage_by_model: Vec::new(),
            usage:          None,
            actor:          None,
        },
    ];

    for terminal_event in terminal_events {
        let state = test_app_state();
        let app = crate::test_support::build_test_router(Arc::clone(&state));
        let run_id = fixtures::RUN_1;
        let (transport, _control_rx) = worker_transport_with_receiver(run_id).await;
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
        let terminal = acp_event_for_stage(&run_id, &terminal_event);
        update_live_run_from_event(&state, run_id, &terminal);

        let req = Request::builder()
            .method("POST")
            .uri(api(&format!("/runs/{run_id}/interrupt")))
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response.into_body()).await;
        assert_eq!(body["errors"][0]["code"], "no_active_steerable_session");
    }
}

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
async fn batch_delete_removes_runs_and_reports_ordered_results() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let first_id = RunId::new();
    let second_id = RunId::new();
    create_succeeded_run(&state, first_id).await;
    create_succeeded_run(&state, second_id).await;

    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/runs/delete",
            &batch_delete_body(&[first_id, second_id], false),
        ))
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["summary"]["requested"], 2);
    assert_eq!(body["summary"]["succeeded"], 2);
    assert_eq!(body["summary"]["failed"], 0);
    let results = body["results"].as_array().unwrap();
    assert_batch_delete_result(&results[0], first_id, true, "deleted");
    assert_batch_delete_result(&results[1], second_id, true, "deleted");

    for run_id in [first_id, second_id] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(api(&format!("/runs/{run_id}")))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_status!(response, StatusCode::NOT_FOUND).await;
    }
}

#[tokio::test]
async fn batch_delete_reports_mixed_results_without_rollback() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let terminal_id = RunId::new();
    let running_id = RunId::new();
    let missing_id = RunId::new();
    create_succeeded_run(&state, terminal_id).await;
    create_running_run(&state, running_id).await;

    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/runs/delete",
            &batch_delete_body(&[terminal_id, running_id, missing_id], false),
        ))
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["summary"]["requested"], 3);
    assert_eq!(body["summary"]["succeeded"], 2);
    assert_eq!(body["summary"]["failed"], 1);
    let results = body["results"].as_array().unwrap();
    assert_batch_delete_result(&results[0], terminal_id, true, "deleted");
    assert_batch_delete_result(&results[1], running_id, false, "conflict");
    assert_eq!(results[1]["error"]["status"], "409");
    assert_batch_delete_result(&results[2], missing_id, true, "already_absent");

    let deleted_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{terminal_id}")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(deleted_response, StatusCode::NOT_FOUND).await;

    let running_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{running_id}")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(running_response, StatusCode::OK).await;
}

#[tokio::test]
async fn batch_delete_force_removes_active_runs() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_running_run(&state, run_id).await;

    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/runs/delete",
            &batch_delete_body(&[run_id], true),
        ))
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["summary"]["requested"], 1);
    assert_eq!(body["summary"]["succeeded"], 1);
    assert_eq!(body["summary"]["failed"], 0);
    let results = body["results"].as_array().unwrap();
    assert_batch_delete_result(&results[0], run_id, true, "deleted");

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
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn batch_delete_with_preserved_sandbox_returns_handoff() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_preserved_local_sandbox_run(&state, run_id).await;

    let response = app
        .clone()
        .oneshot(json_request(
            Method::POST,
            "/runs/delete",
            &batch_delete_body(&[run_id], true),
        ))
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["summary"]["requested"], 1);
    assert_eq!(body["summary"]["succeeded"], 1);
    assert_eq!(body["summary"]["failed"], 0);
    let results = body["results"].as_array().unwrap();
    assert_batch_delete_result(&results[0], run_id, true, "sandbox_preserved");
    assert_eq!(results[0]["sandbox"]["provider"], "local");
    assert_eq!(results[0]["sandbox"]["id"], "sandbox-preserve-1");

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
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn batch_delete_rejects_invalid_requests_before_mutating_runs() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_succeeded_run(&state, run_id).await;
    let too_many_ids = (0..251)
        .map(|_| RunId::new().to_string())
        .collect::<Vec<_>>();
    let invalid_requests = [
        json!({ "run_ids": [], "force": false }),
        json!({ "run_ids": [run_id.to_string(), run_id.to_string()], "force": false }),
        json!({ "run_ids": ["not-a-run-id"], "force": false }),
        json!({ "run_ids": too_many_ids, "force": false }),
    ];

    for body in invalid_requests {
        let response = app
            .clone()
            .oneshot(json_request(Method::POST, "/runs/delete", &body))
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
    assert_status!(response, StatusCode::OK).await;
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
async fn delete_run_removes_durable_run() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let req = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(intent_body(&app, MINIMAL_DOT).await)
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    let run_id = body["id"].as_str().unwrap();

    let req = Request::builder()
        .method("DELETE")
        .uri(api(&format!("/runs/{run_id}?force=true")))
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

#[tokio::test]
async fn delete_run_force_removes_unreadable_durable_run() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_unreadable_durable_run(&state, run_id).await;

    let req = Request::builder()
        .method("GET")
        .uri(api("/system/repair/runs"))
        .body(Body::empty())
        .unwrap();
    let body = response_json!(app.clone().oneshot(req).await.unwrap(), StatusCode::OK).await;
    assert_eq!(body["total_count"], 1);
    assert_eq!(body["runs"][0]["run_id"], run_id.to_string());

    let req = Request::builder()
        .method("DELETE")
        .uri(api(&format!("/runs/{run_id}?force=true")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NO_CONTENT).await;

    let req = Request::builder()
        .method("GET")
        .uri(api("/system/repair/runs"))
        .body(Body::empty())
        .unwrap();
    let body = response_json!(app.oneshot(req).await.unwrap(), StatusCode::OK).await;
    assert_eq!(body["total_count"], 0);
    assert!(body["runs"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn delete_run_without_force_keeps_active_durable_run() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_unreadable_durable_run(&state, run_id).await;

    let req = Request::builder()
        .method("DELETE")
        .uri(api(&format!("/runs/{run_id}")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    response_json!(response, StatusCode::CONFLICT).await;

    let req = Request::builder()
        .method("GET")
        .uri(api("/system/repair/runs"))
        .body(Body::empty())
        .unwrap();
    let body = response_json!(app.oneshot(req).await.unwrap(), StatusCode::OK).await;
    assert_eq!(body["total_count"], 1);
    assert_eq!(body["runs"][0]["run_id"], run_id.to_string());
}

#[tokio::test]
async fn delete_run_with_preserved_sandbox_returns_handoff() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_preserved_local_sandbox_run(&state, run_id).await;

    let req = Request::builder()
        .method("DELETE")
        .uri(api(&format!("/runs/{run_id}?force=true")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["deleted"].as_bool(), Some(true));
    assert_eq!(body["sandbox_preserved"].as_bool(), Some(true));
    assert_eq!(body["sandbox"]["provider"].as_str(), Some("local"));
    assert_eq!(body["sandbox"]["id"].as_str(), Some("sandbox-preserve-1"));
    assert!(body["sandbox"].get("identifier").is_none());

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}")))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn delete_active_run_requires_force() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let req = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(intent_body(&app, MINIMAL_DOT).await)
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    let run_id = body["id"].as_str().unwrap();

    let req = Request::builder()
        .method("DELETE")
        .uri(api(&format!("/runs/{run_id}")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::CONFLICT).await;
    let short_run_id = &run_id[..12.min(run_id.len())];
    let expected = format!(
        "cannot remove active run {short_run_id} (status: submitted, use force=true or --force to force)"
    );
    assert_eq!(
        body["errors"][0]["detail"].as_str(),
        Some(expected.as_str())
    );

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}")))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::OK).await;
}

#[tokio::test]
async fn delete_active_run_force_succeeds() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let req = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(intent_body(&app, MINIMAL_DOT).await)
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    let run_id = body["id"].as_str().unwrap();

    let req = Request::builder()
        .method("DELETE")
        .uri(api(&format!("/runs/{run_id}?force=true")))
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

#[tokio::test]
async fn reconcile_incomplete_runs_terminates_durable_runnable_runs_after_restart() {
    let object_store: Arc<dyn object_store::ObjectStore> =
        Arc::new(object_store::memory::InMemory::new());
    let summaries = fabro_store::test_support::test_run_summary_store();
    let blobs = fabro_store::test_support::test_blob_store();
    let first_state = test_app_state_over_shared_stores(&object_store, &blobs, &summaries);
    let mut histories = Vec::new();

    for (run_id, reason) in [
        (fixtures::RUN_1, FailureReason::Terminated),
        (fixtures::RUN_2, FailureReason::Cancelled),
    ] {
        let mut events = vec![
            workflow_event::Event::RunSubmitted {
                definition_blob: None,
            },
            workflow_event::Event::RunRunnable {
                source: fabro_types::RunRunnableSource::StartRequested,
                actor:  None,
            },
        ];
        let pending_control =
            (reason == FailureReason::Cancelled).then_some(RunControlAction::Cancel);
        if pending_control.is_some() {
            events.push(workflow_event::Event::RunCancelRequested { actor: None });
        }
        create_durable_run_with_events(&first_state, run_id, &events).await;

        let reader = first_state
            .stores
            .runs
            .open_run_reader(&run_id)
            .await
            .unwrap();
        let run = reader.state().await.unwrap();
        assert_eq!(run.status, RunStatus::Runnable);
        assert_eq!(run.pending_control, pending_control);
        let history = reader.list_events().await.unwrap();
        // The fixture may insert intermediate events for later lifecycle states;
        // these runs must remain admitted but never started.
        assert_eq!(history.len(), events.len() + 1);
        assert!(!history.iter().any(|envelope| matches!(
            envelope.event.body,
            EventBody::RunStarting(_) | EventBody::RunRunning(_) | EventBody::RunFailed(_)
        )));
        histories.push((run_id, reason, history));
    }
    assert!(first_state.runs.lock().unwrap().is_empty());
    drop(first_state);

    let reopened_state = test_app_state_over_shared_stores(&object_store, &blobs, &summaries);
    assert!(reopened_state.runs.lock().unwrap().is_empty());
    assert_eq!(
        reconcile_incomplete_runs_on_startup(&reopened_state)
            .await
            .unwrap(),
        2
    );
    assert!(reopened_state.runs.lock().unwrap().is_empty());

    let mut reconciled_histories = Vec::new();
    for (run_id, reason, before) in histories {
        let reader = reopened_state
            .stores
            .runs
            .open_run_reader(&run_id)
            .await
            .unwrap();
        let run = reader.state().await.unwrap();
        assert_eq!(run.status, RunStatus::Failed { reason });
        assert_eq!(run.pending_control, None);
        let summary = summaries.get(&run_id, Utc::now()).await.unwrap().unwrap();
        assert_eq!(summary.lifecycle.status, run.status);
        assert_eq!(summary.lifecycle.pending_control, None);

        let after = reader.list_events().await.unwrap();
        assert_eq!(after.len(), before.len() + 1);
        assert_eq!(&after[..before.len()], before.as_slice());
        assert_eq!(run_failed_reasons(&after), vec![reason]);
        assert!(!after.iter().any(|envelope| matches!(
            envelope.event.body,
            EventBody::RunStarting(_) | EventBody::RunRunning(_)
        )));
        reconciled_histories.push((run_id, after));
    }

    assert_eq!(
        reconcile_incomplete_runs_on_startup(&reopened_state)
            .await
            .unwrap(),
        0
    );
    assert!(reopened_state.runs.lock().unwrap().is_empty());
    for (run_id, expected) in reconciled_histories {
        let reader = reopened_state
            .stores
            .runs
            .open_run_reader(&run_id)
            .await
            .unwrap();
        assert_eq!(reader.list_events().await.unwrap(), expected);
    }
}

#[tokio::test]
async fn reconcile_incomplete_runs_marks_inflight_runs_terminal() {
    let state = test_app_state();

    create_durable_run_with_events(&state, fixtures::RUN_1, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
    ])
    .await;
    create_durable_run_with_events(&state, fixtures::RUN_2, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ])
    .await;
    create_durable_run_with_events(&state, fixtures::RUN_3, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
        workflow_event::Event::RunPaused,
        workflow_event::Event::RunCancelRequested { actor: None },
    ])
    .await;

    create_durable_run_with_events(&state, fixtures::RUN_4, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunPending {
            reason: fabro_types::PendingReason::ApprovalRequired,
            actor:  None,
        },
    ])
    .await;
    let mut untouched_histories = Vec::new();
    for (run_id, expected_status) in [
        (fixtures::RUN_1, RunStatus::Submitted),
        (fixtures::RUN_4, RunStatus::Pending {
            reason: fabro_types::PendingReason::ApprovalRequired,
        }),
    ] {
        let reader = state.stores.runs.open_run_reader(&run_id).await.unwrap();
        assert_eq!(reader.state().await.unwrap().status, expected_status);
        untouched_histories.push((run_id, expected_status, reader.list_events().await.unwrap()));
    }

    let reconciled = reconcile_incomplete_runs_on_startup(&state).await.unwrap();
    assert_eq!(reconciled, 2);

    for (run_id, expected_status, expected_history) in untouched_histories {
        let reader = state.stores.runs.open_run_reader(&run_id).await.unwrap();
        assert_eq!(reader.state().await.unwrap().status, expected_status);
        assert_eq!(reader.list_events().await.unwrap(), expected_history);
        let summary = state
            .stores
            .run_summaries
            .get(&run_id, Utc::now())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(summary.lifecycle.status, expected_status);
    }

    let run_2 = state
        .stores
        .runs
        .open_run_reader(&fixtures::RUN_2)
        .await
        .unwrap()
        .state()
        .await
        .unwrap();
    let run_2_status = run_2.status;
    assert_eq!(run_2_status, RunStatus::Failed {
        reason: FailureReason::Terminated,
    });

    let run_3 = state
        .stores
        .runs
        .open_run_reader(&fixtures::RUN_3)
        .await
        .unwrap()
        .state()
        .await
        .unwrap();
    let run_3_status = run_3.status;
    assert_eq!(run_3_status, RunStatus::Failed {
        reason: FailureReason::Cancelled,
    });
    assert_eq!(run_3.pending_control, None);
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

#[tokio::test]
async fn queue_position_reported_for_runnable_runs() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    // Create and start two runs (no scheduler, both stay runnable)
    let first_run_id = create_and_start_run(&app, MINIMAL_DOT).await;
    let second_run_id = create_and_start_run(&app, MINIMAL_DOT).await;

    // Queue position is tracked in memory even when runnable runs are also
    // visible on the board.
    let runs = state.runs.lock().expect("runs lock poisoned");
    let positions = compute_queue_positions(&runs);
    let first_id = first_run_id.parse::<RunId>().unwrap();
    let second_id = second_run_id.parse::<RunId>().unwrap();
    assert_eq!(positions.get(&first_id).copied(), Some(1));
    assert_eq!(positions.get(&second_id).copied(), Some(2));
}

#[test]
fn scheduler_capacity_counts_only_runs_occupying_slots() {
    assert!(!counts_toward_scheduler_capacity(RunStatus::Submitted));
    assert!(!counts_toward_scheduler_capacity(RunStatus::Pending {
        reason: PendingReason::ApprovalRequired,
    }));
    assert!(!counts_toward_scheduler_capacity(RunStatus::Runnable));
    assert!(counts_toward_scheduler_capacity(RunStatus::Starting));
    assert!(counts_toward_scheduler_capacity(RunStatus::Running));
    assert!(counts_toward_scheduler_capacity(RunStatus::Blocked {
        blocked_reason: BlockedReason::HumanInputRequired,
    }));
    assert!(counts_toward_scheduler_capacity(RunStatus::Paused {
        prior_block: None,
    }));
    assert!(!counts_toward_scheduler_capacity(RunStatus::Removing));
    assert!(!counts_toward_scheduler_capacity(RunStatus::Succeeded {
        reason: SuccessReason::Completed,
    }));
    assert!(!counts_toward_scheduler_capacity(RunStatus::Failed {
        reason: FailureReason::WorkflowError,
    }));
    assert!(!counts_toward_scheduler_capacity(RunStatus::Dead));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrency_limit_respected() {
    let state = test_app_state_with_options(default_test_server_settings(), RunLayer::default(), 1);
    let app = test_app_with_scheduler(Arc::clone(&state));

    // Create and start two runs with max_concurrent_runs=1
    create_and_start_run(&app, MINIMAL_DOT).await;
    create_and_start_run(&app, MINIMAL_DOT).await;

    // Give scheduler time to pick up the first run
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // With max_concurrent_runs=1, at most one run should be live "running".
    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let items = body["data"].as_array().unwrap();
    let active_count = items
        .iter()
        .filter(|item| run_json_status(item)["kind"].as_str() == Some("running"))
        .count();
    assert!(
        active_count <= 1,
        "expected at most 1 active run, got {active_count}"
    );
}
