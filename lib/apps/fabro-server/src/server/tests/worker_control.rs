use super::*;

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
