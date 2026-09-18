use super::*;

#[tokio::test]
async fn system_repair_runs_lists_sql_rows_without_readable_history() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    let run_store = state.stores.runs.create_run(&run_id).await.unwrap();
    append_default_run_created(&run_store, run_id).await;
    state
        .stores
        .run_summaries
        .test_delete_run_events(&run_id)
        .await
        .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(api("/system/repair/runs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["total_count"], 1);
    assert_eq!(body["runs"][0]["run_id"], run_id.to_string());
    let created_at = body["runs"][0]["created_at"]
        .as_str()
        .unwrap()
        .parse::<chrono::DateTime<Utc>>()
        .unwrap();
    assert_eq!(created_at, run_id.created_at());
    assert!(
        body["runs"][0]["error"]
            .as_str()
            .unwrap()
            .contains("head mismatch"),
        "got: {}",
        body["runs"][0]["error"]
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
