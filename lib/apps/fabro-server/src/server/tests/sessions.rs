use super::*;

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
async fn get_run_state_exposes_pending_interviews() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = fixtures::RUN_1;

    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ])
    .await;
    append_raw_run_event(
        &state,
        run_id,
        "pending-question",
        "2026-04-19T12:00:00Z",
        "interview.started",
        json!({
            "question_id": "q-1",
            "question": "Approve deploy?",
            "stage": "gate",
            "question_type": "multiple_choice",
            "options": [],
            "allow_freeform": false,
            "context_display": null,
            "timeout_seconds": null,
            "review_target": {
                "label": "Quarry review exercise",
                "url": "https://quarry.lithos.computer/tmp/0123456789abcdef0123456789abcdef",
                "kind": "document"
            },
        }),
        Some("gate"),
    )
    .await;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/state")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(
        body["pending_interviews"]["q-1"]["question"]["text"].as_str(),
        Some("Approve deploy?")
    );
    assert_eq!(
        body["pending_interviews"]["q-1"]["question"]["stage"].as_str(),
        Some("gate")
    );
}

#[tokio::test]
async fn inspects_worker_creates_ask_session_only_on_declared_workflow_runs() {
    let (state, app) = jwt_auth_app();
    let parent_run_id = unique_run_id();
    let develop_run_id = unique_run_id();
    let nightly_run_id = unique_run_id();
    create_run_with_workflow_slug(&state, develop_run_id, "develop").await;
    create_run_with_workflow_slug(&state, nightly_run_id, "nightly").await;
    let inspects_token = issue_test_inspects_worker_token(&parent_run_id, &["develop".to_string()]);
    {
        let keys = crate::worker_token::WorkerTokenKeys::from_master_secret(
            TEST_SESSION_SECRET.as_bytes(),
        )
        .unwrap();
        let own =
            crate::worker_token::decode_worker_token(&inspects_token, &keys).expect("CONTROL 0a");
        let _ = own;
        let state_decode =
            crate::worker_token::decode_worker_token(&inspects_token, state.worker_token_keys());
        let _ = state_decode.expect("CONTROL 0b: jwt_auth_app state keys decode");
    }
    let plain_run_tools_token = issue_test_run_tools_worker_token(&parent_run_id);

    // Declared workflow: session creation succeeds.
    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            &format!("/runs/{develop_run_id}/sessions"),
            &inspects_token,
            &json!({ "title": "revisor question" }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::CREATED).await;

    // Foreign workflow: rejected even though the token is otherwise valid.
    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            &format!("/runs/{nightly_run_id}/sessions"),
            &inspects_token,
            &json!({ "title": "revisor question" }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    // Run-tools workers without inspects authority keep being rejected.
    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            &format!("/runs/{develop_run_id}/sessions"),
            &plain_run_tools_token,
            &json!({ "title": "revisor question" }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    // A slug-less run is never in scope.
    let user_jwt = issue_test_user_jwt();
    let manifest_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            &format!("/runs/{manifest_run_id}/sessions"),
            &inspects_token,
            &json!({ "title": "revisor question" }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;
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
