use super::*;

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
async fn run_tools_worker_reads_pair_status_and_transcript_for_created_runs_only() {
    // fabro-4556: pair routes share the run-management inspection scope —
    // foreign targets are 403, runs the worker created stay readable.
    let (state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let origin_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let foreign_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_run_tools_worker_token(&origin_run_id);

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{foreign_run_id}/pair"),
            &worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    let target_run_id = create_run_with_bearer(&app, &worker_token).await;
    let pair_id = append_pair_transcript_fixture(&state, target_run_id).await;

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
    let status_body = response_json!(response, StatusCode::OK).await;
    assert_eq!(status_body["run_id"], target_run_id.to_string());

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{target_run_id}/pair/{pair_id}/transcript"),
            &worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    let transcript_body = response_json!(response, StatusCode::OK).await;
    assert_eq!(transcript_body["data"].as_array().unwrap().len(), 1);
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
