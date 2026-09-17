use super::*;

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
async fn list_run_stages_shows_retrying_after_failed_event() {
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
    append_scoped_stage_event(
        &state,
        run_id,
        "work",
        1,
        &workflow_event::Event::StageRetrying {
            node_id:      "work".to_string(),
            name:         "Work".to_string(),
            index:        0,
            attempt:      2,
            max_attempts: 3,
            delay_ms:     50,
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

#[tokio::test]
async fn get_run_status_advances_live_active_timing_between_events() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
        workflow_run_started_event(run_id),
    ])
    .await;
    append_scoped_stage_event(
        &state,
        run_id,
        "work",
        1,
        &stage_started_event("work", "command"),
    )
    .await;

    // The SQLite summary stores timing at the StageStarted event. A later
    // detail read must overlay the in-flight command's active time from the
    // projection even though no newer event has arrived.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
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
    let timing = &body["timing"];

    assert!(timing["active_time_ms"].as_u64().unwrap() > 0);
    assert_eq!(timing["tool_time_ms"], timing["active_time_ms"]);
    assert!(timing["wall_time_ms"].as_u64().unwrap() >= timing["active_time_ms"].as_u64().unwrap());
}

#[tokio::test]
async fn get_events_not_found() {
    let app = test_app_with();
    let missing_run_id = fixtures::RUN_64;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{missing_run_id}/events")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn run_projection_endpoints_reflect_events_appended_to_an_open_run() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();

    let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
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
    append_scoped_stage_event(
        &state,
        run_id,
        "review",
        1,
        &workflow_event::Event::StageStarted {
            graph_visit:           None,
            resumed_from_stage_id: None,
            node_id:               "review".to_string(),
            name:                  "Review".to_string(),
            index:                 0,
            handler_type:          "human".to_string(),
            attempt:               1,
            max_attempts:          1,
        },
    )
    .await;
    append_raw_run_event(
        &state,
        run_id,
        "cache-question",
        "2026-04-19T12:00:00Z",
        "interview.started",
        json!({
            "question_id": "q-cache",
            "question": "Approve cached deploy?",
            "stage": "review",
            "question_type": "yes_no",
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
        Some("review"),
    )
    .await;
    append_raw_run_event(
        &state,
        run_id,
        "cache-checkpoint",
        "2026-04-19T12:00:01Z",
        "checkpoint.completed",
        json!({
            "status": "running",
            "current_node": "review",
            "completed_nodes": [],
            "node_retries": {},
            "context_values": {},
            "node_outcomes": {},
            "next_node_id": "review",
            "git_commit_sha": "cache-sha",
            "loop_failure_signatures": {},
            "restart_failure_signatures": {},
            "node_visits": { "review": 1 },
        }),
        Some("review"),
    )
    .await;

    let status = app
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
    let status = response_json!(status, StatusCode::OK).await;
    assert_eq!(run_json_status(&status)["kind"].as_str(), Some("running"));

    let state_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/state")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let state_body = response_json!(state_response, StatusCode::OK).await;
    assert_eq!(
        state_body["pending_interviews"]["q-cache"]["question"]["text"].as_str(),
        Some("Approve cached deploy?")
    );
    assert_eq!(
        state_body["pending_interviews"]["q-cache"]["question"]["review_target"]["url"].as_str(),
        Some("https://quarry.lithos.computer/tmp/0123456789abcdef0123456789abcdef")
    );

    let stages = app
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
    let stages = response_json!(stages, StatusCode::OK).await;
    assert_eq!(stages["data"][0]["id"].as_str(), Some("review@1"));

    let questions = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/questions")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let questions = response_json!(questions, StatusCode::OK).await;
    assert_eq!(
        questions["data"][0]["text"].as_str(),
        Some("Approve cached deploy?")
    );
    assert_eq!(
        questions["data"][0]["review_target"]["label"].as_str(),
        Some("Quarry review exercise")
    );

    let settings = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/settings")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(settings, StatusCode::OK).await;

    let checkpoint = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/checkpoint")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let checkpoint = response_json!(checkpoint, StatusCode::OK).await;
    assert_eq!(checkpoint["git_commit_sha"].as_str(), Some("cache-sha"));

    let usage = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/usage")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let usage = response_json!(usage, StatusCode::OK).await;
    assert_eq!(usage["stages"][0]["stage"]["id"].as_str(), Some("review"));
}

#[tokio::test]
async fn list_run_events_returns_paginated_json() {
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
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/events?since_seq=1&limit=5")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert!(body["data"].is_array());
    assert!(body["meta"]["has_more"].is_boolean());
}

#[tokio::test]
async fn list_run_events_descends_from_latest_with_exclusive_cursor() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunRunnable {
            source: fabro_types::RunRunnableSource::StartRequested,
            actor:  None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ])
    .await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/events?order=desc&limit=2")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let seqs = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["seq"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(seqs, vec![4, 3]);
    assert_eq!(body["meta"]["has_more"], true);

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!(
                    "/runs/{run_id}/events?order=desc&before_seq=3&limit=2"
                )))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let seqs = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["seq"].as_u64().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(seqs, vec![2, 1]);
    assert_eq!(body["meta"]["has_more"], false);
}

#[tokio::test]
async fn list_run_events_rejects_cursor_for_opposite_order() {
    let app = crate::test_support::build_test_router(test_app_state());
    let run_id = RunId::new();
    let cases = [
        (
            format!("/runs/{run_id}/events?order=desc&since_seq=2"),
            "since_seq cannot be combined with order=desc; use before_seq instead.",
        ),
        (
            format!("/runs/{run_id}/events?before_seq=2"),
            "before_seq requires order=desc.",
        ),
    ];

    for (path, expected_detail) in cases {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(api(&path))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response_json!(response, StatusCode::BAD_REQUEST).await;
        assert_eq!(body["errors"][0]["detail"], expected_detail);
    }
}

#[tokio::test]
async fn append_run_event_rejects_run_id_mismatch() {
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
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/events")))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({
                "id": "evt-test",
                "ts": "2026-03-27T12:00:00Z",
                "run_id": fixtures::RUN_64.to_string(),
                "event": "run.submitted",
                "properties": {}
            })
            .to_string(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn append_run_event_accepts_a_body_larger_than_two_mib() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_run(&app, MINIMAL_DOT).await;
    let payload = json!({
        "id": "evt-large-agent-output",
        "ts": "2026-08-24T12:00:00Z",
        "run_id": run_id,
        "event": "agent.tool.completed",
        "properties": {
            "stage": "code",
            "visit": 1,
            "session_id": "ses_large",
            "timestamp": "2026-08-24T12:00:00.000Z",
            "event": {
                "ToolCallCompleted": {
                    "tool_name": "shell",
                    "tool_call_id": "call-large",
                    "output": "x".repeat(2 * 1024 * 1024),
                    "is_error": false,
                    "output_bytes_observed": 2 * 1024 * 1024,
                    "output_bytes_retained": 2 * 1024 * 1024,
                    "output_bytes_omitted": 0
                }
            }
        }
    })
    .to_string();
    assert!(payload.len() > 2 * 1024 * 1024);
    assert!(payload.len() < 3 * 1024 * 1024);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/events")))
                .header("content-type", "application/json")
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_status!(response, StatusCode::OK).await;
}

#[tokio::test]
async fn append_run_event_rejects_a_body_larger_than_three_mib() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_run(&app, MINIMAL_DOT).await;
    let payload = json!({
        "id": "evt-oversized-agent-output",
        "ts": "2026-08-24T12:00:00Z",
        "run_id": run_id,
        "event": "agent.tool.completed",
        "properties": {
            "tool_name": "shell",
            "tool_call_id": "call-oversized",
            "output": "x".repeat(3 * 1024 * 1024),
            "is_error": false,
            "visit": 1
        }
    })
    .to_string();
    assert!(payload.len() > 3 * 1024 * 1024);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/events")))
                .header("content-type", "application/json")
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_status!(response, StatusCode::PAYLOAD_TOO_LARGE).await;
}

#[tokio::test]
async fn append_run_event_rejects_reserved_archive_event() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_run(&app, MINIMAL_DOT).await;

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!("/runs/{run_id}/events")))
        .header("content-type", "application/json")
        .body(Body::from(
            json!({
                "id": "evt-run-archived",
                "ts": "2026-04-19T12:00:00Z",
                "run_id": run_id,
                "event": "run.archived",
                "properties": {
                    "actor": null
                }
            })
            .to_string(),
        ))
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::BAD_REQUEST).await;
    assert!(
        body["errors"][0]["detail"].as_str().is_some_and(|message| {
            message
                .contains("run.archived must be performed through its dedicated operation endpoint")
        }),
        "expected dedicated-operation rejection, got: {body}"
    );
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

#[tokio::test]
async fn filtered_global_events_streams_only_matching_run_ids() {
    let run_one = fixtures::RUN_1;
    let run_two = fixtures::RUN_2;
    let (event_tx, _) = broadcast::channel(8);

    let stream = filtered_global_events(event_tx.subscribe(), Some(HashSet::from([run_one])));

    event_tx
        .send(test_event_envelope(
            1,
            run_two,
            EventBody::RunRunnable(fabro_types::run_event::RunRunnableProps {
                source: fabro_types::RunRunnableSource::StartRequested,
            }),
        ))
        .unwrap();
    event_tx
        .send(test_event_envelope(
            2,
            run_one,
            EventBody::RunRunnable(fabro_types::run_event::RunRunnableProps {
                source: fabro_types::RunRunnableSource::StartRequested,
            }),
        ))
        .unwrap();
    drop(event_tx);

    let events = stream.collect::<Vec<_>>().await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].seq, 2);
    assert_eq!(events[0].event.run_id, run_one);
}
