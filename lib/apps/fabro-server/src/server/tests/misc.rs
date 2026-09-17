use super::*;

#[tokio::test]
async fn github_webhook_rejects_missing_signature() {
    let app = webhook_test_app(crate::test_support::test_auth_mode());
    let body = br#"{"action":"opened"}"#;

    let response = app
        .oneshot(webhook_request(None, None, body))
        .await
        .unwrap();
    assert_status!(response, StatusCode::UNAUTHORIZED).await;
}

#[tokio::test]
async fn github_webhook_accepts_valid_signature_without_bearer_token() {
    let body = br#"{"repository":{"full_name":"owner/repo"},"action":"opened"}"#;
    let signature = compute_signature(TEST_WEBHOOK_SECRET.as_bytes(), body);
    let app = webhook_test_app(dev_token_auth_mode());

    let response = app
        .oneshot(webhook_request(Some(&signature), None, body))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;
}

#[tokio::test]
async fn github_webhook_accepts_valid_signature_with_wrong_bearer_token() {
    let body = br#"{"repository":{"full_name":"owner/repo"},"action":"opened"}"#;
    let signature = compute_signature(TEST_WEBHOOK_SECRET.as_bytes(), body);
    let app = webhook_test_app(dev_token_auth_mode());

    let response = app
        .oneshot(webhook_request(
            Some(&signature),
            Some(&format!("Bearer {WRONG_DEV_TOKEN}")),
            body,
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;
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

#[cfg(unix)]
#[tokio::test]
async fn render_dot_subprocess_returns_child_crashed_for_nonzero_exit() {
    let (_dir, script_path) = write_test_executable("#!/bin/sh\nexit 1\n");

    let result = render_dot_subprocess("digraph { a -> b }", Some(&script_path)).await;

    assert!(matches!(
        result,
        Err(RenderSubprocessError::ChildCrashed(_))
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn render_dot_subprocess_returns_protocol_violation_for_garbage_stdout() {
    let (_dir, script_path) =
        write_test_executable("#!/bin/sh\ncat >/dev/null\nprintf 'garbage'\nexit 0\n");

    let result = render_dot_subprocess("digraph { a -> b }", Some(&script_path)).await;

    assert!(matches!(
        result,
        Err(RenderSubprocessError::ProtocolViolation(_))
    ));
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
