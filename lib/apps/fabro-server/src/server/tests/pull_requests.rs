use super::*;

#[tokio::test]
async fn post_runs_run_intent_rejects_automatic_pull_requests_for_local_environment() {
    let target = tempfile::tempdir().unwrap();
    let state = local_test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let workflow_version_id = store_workflow_version(
        &state,
        MINIMAL_DOT,
        Some("_version = 1\n[run.pull_request]\nenabled = true\n"),
    )
    .await;

    let response =
        post_run_intent_response(&app, folder_intent(workflow_version_id, target.path())).await;
    let body = response_json!(response, StatusCode::UNPROCESSABLE_ENTITY).await;

    assert_eq!(
        body["errors"][0]["code"],
        "pull_request_environment_unsupported"
    );
    assert_eq!(
        body["errors"][0]["detail"],
        "automatic pull requests require a clone-based Docker or Daytona environment; disable run.pull_request.enabled for Local execution"
    );
    assert!(state.runs.lock().expect("runs lock poisoned").is_empty());
    assert!(
        state
            .stores
            .run_summaries
            .list_identities()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn post_runs_run_intent_accepts_disabled_pull_requests_for_local_environment() {
    let target = tempfile::tempdir().unwrap();
    let state = local_test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let workflow_version_id = store_workflow_version(
        &state,
        MINIMAL_DOT,
        Some("_version = 1\n[run.pull_request]\nenabled = false\n"),
    )
    .await;

    // `post_run_intent` asserts the `201 Created` admission outcome.
    post_run_intent(&app, folder_intent(workflow_version_id, target.path())).await;
}

#[tokio::test]
async fn post_runs_run_intent_accepts_automatic_pull_requests_for_configured_docker_dry_run() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let workflow_version_id = store_workflow_version(
        &state,
        MINIMAL_DOT,
        Some("_version = 1\n[run.pull_request]\nenabled = true\n"),
    )
    .await;

    let body = post_run_intent(
        &app,
        json!({
            "workflow_version_id": workflow_version_id,
            "target": {
                "kind": "git",
                "repo": "fabro-sh/fabro",
                "branch": "main"
            },
            "args": { "dry_run": true }
        }),
    )
    .await;
    let run_id = body["id"].as_str().unwrap().parse::<RunId>().unwrap();
    let projection = state
        .stores
        .runs
        .open_run_reader(&run_id)
        .await
        .unwrap()
        .state()
        .await
        .unwrap();

    assert_eq!(
        projection.spec.settings.run.environment.provider,
        SandboxProviderKind::DOCKER
    );
    assert_eq!(projection.spec.settings.run.execution.mode, RunMode::DryRun);
    assert!(projection.spec.settings.run.pull_request.is_some());
}

#[tokio::test]
async fn get_run_pull_request_returns_live_detail_from_github() {
    let github = MockServer::start();
    let github_mock = github.mock(|when, then| {
        when.method("GET")
            .path("/repos/acme/widgets/pulls/42")
            .header("authorization", "Bearer ghu_test");
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "number": 42,
                    "title": "Fix the bug",
                    "body": "Detailed description",
                    "state": "closed",
                    "draft": false,
                    "merged": true,
                    "merged_at": "2026-04-23T15:45:00Z",
                    "mergeable": false,
                    "additions": 10,
                    "deletions": 3,
                    "changed_files": 2,
                    "html_url": "https://github.com/acme/widgets/pull/42",
                    "user": { "login": "testuser" },
                    "head": { "ref": "feature" },
                    "base": { "ref": "main" },
                    "created_at": "2026-04-23T15:40:00Z",
                    "updated_at": "2026-04-23T15:45:00Z"
                })
                .to_string(),
            );
    });
    let (state, app, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));

    create_run_with_pull_request_record(
        &state,
        run_id,
        "https://github.com/acme/widgets/pull/42",
        42,
        "Fix the bug",
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;

    assert_eq!(body["data"]["link"]["number"], 42);
    assert_eq!(body["data"]["link"]["owner"], "acme");
    assert_eq!(body["data"]["details"]["state"], "closed");
    assert_eq!(body["data"]["details"]["merged"], true);
    assert_eq!(body["data"]["details"]["head_branch"], "feature");
    assert_eq!(body["data"]["details"]["base_branch"], "main");
    assert_eq!(body["meta"]["details_status"], "available");
    github_mock.assert();
}

#[tokio::test]
async fn get_run_pull_request_returns_not_found_when_record_missing() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_run(&app, MINIMAL_DOT).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::NOT_FOUND).await;

    assert_eq!(body["errors"][0]["code"], "no_stored_record");
}

#[tokio::test]
async fn link_run_pull_request_links_github_pr_from_any_repo_and_updates_state() {
    let (_state, app, run_id) = pr_test_app_with_minimal_run(None, None).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "html_url": "https://github.com/other/repo/pull/987"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;

    assert_eq!(body["html_url"], "https://github.com/other/repo/pull/987");
    assert_eq!(body["owner"], "other");
    assert_eq!(body["repo"], "repo");
    assert_eq!(body["number"], 987);

    let state_response = app
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
        state_body["pull_request"]["html_url"],
        "https://github.com/other/repo/pull/987"
    );
}

#[tokio::test]
async fn link_run_pull_request_rejects_non_github_url() {
    let (_state, app, run_id) = pr_test_app_with_minimal_run(None, None).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "html_url": "https://gitlab.com/acme/widgets/-/merge_requests/42"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::BAD_REQUEST).await;

    assert_eq!(
        body["errors"][0]["code"],
        "unsupported_pull_request_provider"
    );
}

#[tokio::test]
async fn unlink_run_pull_request_appends_event_and_clears_projected_state() {
    let (state, app, run_id) = pr_test_app_with_minimal_run(None, None).await;
    let link_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "html_url": "https://github.com/acme/widgets/pull/42"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    response_json!(link_response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;

    assert_eq!(body["html_url"], "https://github.com/acme/widgets/pull/42");

    let state_response = app
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
    assert!(state_body["pull_request"].is_null());

    let run_id = run_id.parse::<RunId>().unwrap();
    let run_store = state.stores.runs.open_run_reader(&run_id).await.unwrap();
    let events = run_store.list_events().await.unwrap();
    assert!(events.iter().any(|event| {
        event.event.event_name() == "pull_request.unlinked"
            && event.event.properties().unwrap()["pull_request"]["html_url"]
                == "https://github.com/acme/widgets/pull/42"
    }));
}

#[tokio::test]
async fn get_run_pull_request_returns_stored_github_association_without_github_credentials() {
    let (state, app, run_id) = pr_test_app(None, None);

    create_run_with_pull_request_record(
        &state,
        run_id,
        "https://github.com/acme/widgets/pull/42",
        42,
        "Fix the bug",
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;

    assert_eq!(body["data"]["link"]["number"], 42);
    assert_eq!(
        body["data"]["link"]["html_url"],
        "https://github.com/acme/widgets/pull/42"
    );
    assert!(body["data"]["details"].is_null());
    assert_eq!(body["meta"]["details_status"], "unavailable");
    assert_eq!(
        body["meta"]["details_unavailable_reason"],
        "integration_unavailable"
    );
}

#[tokio::test]
async fn get_run_pull_request_returns_stored_github_association_when_github_pr_is_missing() {
    let github = MockServer::start();
    let github_mock = github.mock(|when, then| {
        when.method("GET")
            .path("/repos/acme/widgets/pulls/42")
            .header("authorization", "Bearer ghu_test");
        then.status(404)
            .header("content-type", "application/json")
            .body(json!({ "message": "Not Found" }).to_string());
    });
    let (state, app, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));

    create_run_with_pull_request_record(
        &state,
        run_id,
        "https://github.com/acme/widgets/pull/42",
        42,
        "Fix the bug",
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;

    assert_eq!(body["data"]["link"]["number"], 42);
    assert_eq!(
        body["data"]["link"]["html_url"],
        "https://github.com/acme/widgets/pull/42"
    );
    assert!(body["data"]["details"].is_null());
    assert_eq!(body["meta"]["details_status"], "unavailable");
    assert_eq!(body["meta"]["details_unavailable_reason"], "not_found");
    github_mock.assert();
}

#[tokio::test]
async fn pull_request_creation_recovers_durable_request_after_crash_gap() {
    let github = MockServer::start();
    let branch_mock = github.mock(|when, then| {
        when.method("GET")
            .path("/repos/acme/widgets/branches/fabro/run/42")
            .header("authorization", "Bearer ghu_test");
        then.status(200)
            .header("content-type", "application/json")
            .body(json!({ "commit": { "sha": "final-sha" } }).to_string());
    });
    let create_mock = github.mock(|when, then| {
        when.method("POST")
            .path("/repos/acme/widgets/pulls")
            .header("authorization", "Bearer ghu_test");
        then.status(201)
            .header("content-type", "application/json")
            .body(
                json!({
                    "html_url": "https://github.com/acme/widgets/pull/42",
                    "number": 42,
                    "node_id": "PR_kwDOAA"
                })
                .to_string(),
            );
    });
    let find_mock = github.mock(|when, then| {
        when.method("GET")
            .path("/repos/acme/widgets/pulls")
            .query_param("state", "open")
            .query_param("base", "main")
            .query_param("head", "acme:fabro/run/42")
            .header("authorization", "Bearer ghu_test");
        then.status(200)
            .header("content-type", "application/json")
            .body("[]");
    });
    let llm = MockServer::start_async().await;
    let response_mock = llm
        .mock_async(|when, then| {
            when.method(POST)
                .path("/v1/responses")
                .header("authorization", "Bearer openai-key");
            then.status(200)
                .header("content-type", "application/json")
                .json_body(openai_responses_payload(
                    &serde_json::to_string(&json!({
                        "title": "Mock title",
                        "body": "Narrative from mock.",
                    }))
                    .unwrap(),
                ));
        })
        .await;
    let state = create_github_token_app_state_with_env_lookup_and_llm_catalog_settings(
        Some("ghu_test"),
        Some(github.base_url()),
        |_| None,
        llm_overlay_with_provider_base_url("openai", llm.url("/v1")),
    );
    state
        .stores
        .vault
        .set("OPENAI_API_KEY", "openai-key", SecretType::Token, None)
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = fixtures::RUN_1;
    Box::pin(create_completed_run_ready_for_pull_request(
        &state,
        run_id,
        Some("git@github.com:acme/widgets.git"),
        Some("main"),
        Some("fabro/run/42"),
        "diff --git a/src/lib.rs b/src/lib.rs\n+fn shipped() {}\n",
    ))
    .await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "force": false,
                        "model": "gpt-5.4"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        &format!("/api/v1/runs/{run_id}/pull_request/creation")
    );
    let body = response_json!(response, StatusCode::ACCEPTED).await;

    assert_eq!(body["status"], "pending");
    assert_eq!(body["model"], "gpt-5.4");
    assert_eq!(state.pull_request_creation_queue_len(), 1);

    // Starting the supervisor after the request simulates server recovery:
    // the durable pending event is enough to resume the operation.
    let _ = state.drain_pull_request_creation_queue();
    let supervisor = spawn_pull_request_creation_supervisor(Arc::clone(&state));

    let creation_body = wait_for_pull_request_creation(&app, run_id).await;

    assert_eq!(creation_body["status"], "succeeded");
    assert_eq!(creation_body["pull_request"]["number"], 42);
    assert_eq!(creation_body["pull_request"]["owner"], "acme");
    assert_eq!(creation_body["pull_request"]["repo"], "widgets");
    assert_eq!(
        creation_body["pull_request"]["html_url"],
        "https://github.com/acme/widgets/pull/42"
    );

    let state_response = app
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
    assert_eq!(state_body["pull_request"]["number"], 42);
    assert_eq!(state_body["pull_request"]["owner"], "acme");
    assert_eq!(state_body["pull_request"]["repo"], "widgets");

    response_mock.assert_async().await;
    branch_mock.assert();
    find_mock.assert();
    create_mock.assert();
    state.shutdown_token().cancel();
    supervisor.await.unwrap();
}

#[tokio::test]
async fn pull_request_creation_returns_the_active_durable_request() {
    let github = MockServer::start();
    let (state, app, run_id) = Box::pin(pr_test_app_with_completed_run(
        Some("ghu_test"),
        Some(github.base_url()),
        Some("https://github.com/acme/widgets.git"),
    ))
    .await;

    let configured_provider_ids = state
        .ready_llm_provider_ids()
        .await
        .into_iter()
        .collect::<HashSet<_>>();
    let expected_default_model = state
        .catalog()
        .default_offering_for(&configured_provider_ids)
        .expect("a ready provider should have a default model")
        .model
        .id()
        .to_string();
    let request_body = json!({
        "force": false,
        "model": null
    })
    .to_string();
    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .header("content-type", "application/json")
                .body(Body::from(request_body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    let first_body = response_json!(first, StatusCode::ACCEPTED).await;

    let second = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .header("content-type", "application/json")
                .body(Body::from(request_body))
                .unwrap(),
        )
        .await
        .unwrap();
    let second_body = response_json!(second, StatusCode::ACCEPTED).await;

    assert_eq!(first_body["id"], second_body["id"]);
    assert_eq!(first_body["model"], expected_default_model);
    assert_eq!(state.pull_request_creation_queue_len(), 1);
    let run_store = state.stores.runs.open_run_reader(&run_id).await.unwrap();
    let events = run_store.list_events().await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event.event_name() == "pull_request.creation_requested")
            .count(),
        1
    );
}

#[tokio::test]
async fn pull_request_creation_queue_overflow_recovers_from_indexed_scan() {
    let state = test_app_state();
    let mut creation_ids = HashMap::new();
    for _ in 0..17 {
        let run_id = RunId::new();
        let creation_id = fabro_types::PullRequestCreationId::new();
        creation_ids.insert(run_id, creation_id);
        create_durable_run_with_events(&state, run_id, &[
            workflow_event::Event::PullRequestCreationRequested {
                creation_id,
                model: "test-model".to_string(),
                force: false,
            },
        ])
        .await;
    }

    pull_request_supervisor::recover_pending_pull_request_creations(
        state.as_ref(),
        &HashMap::new(),
        &HashMap::new(),
    )
    .await
    .unwrap();
    let first_batch = state.drain_pull_request_creation_queue();
    assert_eq!(first_batch.len(), 16);

    for run_id in first_batch {
        let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
        workflow_event::append_event(
            &run_store,
            &run_id,
            &workflow_event::Event::PullRequestFailed {
                creation_id: creation_ids.get(&run_id).copied(),
                error:       "test failure".to_string(),
            },
        )
        .await
        .unwrap();
    }

    pull_request_supervisor::recover_pending_pull_request_creations(
        state.as_ref(),
        &HashMap::new(),
        &HashMap::new(),
    )
    .await
    .unwrap();
    let recovered = state.drain_pull_request_creation_queue();
    assert_eq!(recovered.len(), 1);
    let recovered_id = recovered[0];
    let projection = state
        .stores
        .runs
        .open_run_reader(&recovered_id)
        .await
        .unwrap()
        .state()
        .await
        .unwrap();
    assert!(
        projection
            .pull_request_creation
            .as_ref()
            .is_some_and(fabro_types::PullRequestCreation::is_pending)
    );
}

#[tokio::test]
async fn pull_request_creation_persists_generation_failure() {
    let github = MockServer::start();
    let branch_mock = github.mock(|when, then| {
        when.method("GET")
            .path("/repos/acme/widgets/branches/fabro/run/42")
            .header("authorization", "Bearer ghu_test");
        then.status(200)
            .header("content-type", "application/json")
            .body(json!({ "commit": { "sha": "final-sha" } }).to_string());
    });
    let find_mock = github.mock(|when, then| {
        when.method("GET")
            .path("/repos/acme/widgets/pulls")
            .query_param("state", "open")
            .query_param("base", "main")
            .query_param("head", "acme:fabro/run/42")
            .header("authorization", "Bearer ghu_test");
        then.status(200)
            .header("content-type", "application/json")
            .body("[]");
    });
    let (state, app, run_id) = Box::pin(pr_test_app_with_completed_run(
        Some("ghu_test"),
        Some(github.base_url()),
        Some("https://github.com/acme/widgets.git"),
    ))
    .await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "force": false, "model": "gpt-5.4" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    response_json!(response, StatusCode::ACCEPTED).await;
    let supervisor = spawn_pull_request_creation_supervisor(Arc::clone(&state));

    let creation = wait_for_pull_request_creation(&app, run_id).await;

    // Since the deterministic PR-content fallback, an LLM generation failure
    // no longer fails the creation: the PR is published with the
    // goal-derived title. This fixture stops at the unmocked GitHub PR
    // creation route, so the failure we persist is the GitHub 404 — the
    // point is that the LLM error did NOT abort the publish flow earlier.
    assert_eq!(creation["status"], "failed");
    assert!(
        creation["error"]
            .as_str()
            .is_some_and(|error| !error.contains("LLM generation failed")),
        "LLM failure must not abort the publish flow anymore: {:?}",
        creation["error"]
    );
    assert!(creation["pull_request"].is_null());
    branch_mock.assert();
    // Fallback title flows into create_pull_request, which 404s on the
    // unmocked route, then reconcile runs once more (before + after the
    // failed create) — the LLM failure no longer aborts before GitHub.
    assert_eq!(find_mock.calls_async().await, 2);
    state.shutdown_token().cancel();
    supervisor.await.unwrap();
}

#[tokio::test]
async fn create_run_pull_request_returns_conflict_when_record_exists() {
    let (state, app, run_id) = pr_test_app(None, None);

    create_run_with_pull_request_record(
        &state,
        run_id,
        "https://github.com/acme/widgets/pull/42",
        42,
        "Fix the bug",
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "force": false, "model": null }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::CONFLICT).await;

    assert_eq!(body["errors"][0]["code"], "pull_request_exists");
    assert!(
        body["errors"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("https://github.com/acme/widgets/pull/42")
    );
}

#[tokio::test]
async fn pull_request_creation_rejects_missing_repo_origin_without_enqueue() {
    let (state, app, run_id) = Box::pin(pr_test_app_with_completed_run(None, None, None)).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "force": false,
                        "model": "claude-sonnet-4-6"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::BAD_REQUEST).await;

    assert_eq!(body["errors"][0]["code"], "missing_repo_origin");
    assert_eq!(state.pull_request_creation_queue_len(), 0);
}

#[tokio::test]
async fn pull_request_creation_rejects_missing_credentials_without_enqueue() {
    let (state, app, run_id) = Box::pin(pr_test_app_with_completed_run(
        None,
        None,
        Some("https://github.com/acme/widgets.git"),
    ))
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "force": false,
                        "model": "claude-sonnet-4-6"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::SERVICE_UNAVAILABLE).await;

    assert_eq!(state.pull_request_creation_queue_len(), 0);

    assert_eq!(body["errors"][0]["code"], "integration_unavailable");
}

#[tokio::test]
async fn create_run_pull_request_rejects_non_github_origin_url() {
    let (_state, app, run_id) = Box::pin(pr_test_app_with_completed_run(
        Some("ghu_test"),
        None,
        Some("https://gitlab.com/acme/widgets.git"),
    ))
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "force": false,
                        "model": "claude-sonnet-4-6"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::BAD_REQUEST).await;

    assert_eq!(body["errors"][0]["code"], "unsupported_host");
}

#[tokio::test]
async fn pull_request_endpoints_use_github_base_url_captured_at_startup() {
    let github = MockServer::start();
    let captured_mock = github.mock(|when, then| {
        when.method("GET")
            .path("/repos/acme/widgets/pulls/42")
            .header("authorization", "Bearer ghu_test");
        then.status(200)
            .header("content-type", "application/json")
            .body(
                json!({
                    "number": 42,
                    "title": "Captured",
                    "body": "",
                    "state": "open",
                    "draft": false,
                    "merged": false,
                    "mergeable": true,
                    "additions": 1,
                    "deletions": 0,
                    "changed_files": 1,
                    "html_url": "https://github.com/acme/widgets/pull/42",
                    "user": { "login": "octocat" },
                    "head": { "ref": "feature" },
                    "base": { "ref": "main" },
                    "created_at": "2026-04-23T12:00:00Z",
                    "updated_at": "2026-04-23T12:00:00Z"
                })
                .to_string(),
            );
    });
    let state = create_github_token_app_state(Some("ghu_test"), Some(github.base_url()));
    assert_eq!(state.github_api_base_url, github.base_url());

    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = fixtures::RUN_1;
    create_run_with_pull_request_record(
        &state,
        run_id,
        "https://github.com/acme/widgets/pull/42",
        42,
        "Captured",
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/pull_request")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    response_json!(response, StatusCode::OK).await;

    // If the handler read GITHUB_BASE_URL at request time instead of using the
    // value captured at AppState construction, the outbound call would miss
    // this mock — no other server is running at the captured URL, and the
    // process env default points elsewhere.
    captured_mock.assert();
}

#[tokio::test]
async fn merge_run_pull_request_returns_not_found_when_record_missing() {
    let (_state, app, run_id) = pr_test_app_with_minimal_run(Some("ghu_test"), None).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/pull_request/merge")))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "method": "squash" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::NOT_FOUND).await;

    assert_eq!(body["errors"][0]["code"], "no_stored_record");
}

#[tokio::test]
async fn merge_run_pull_request_rejects_invalid_method() {
    let (state, app, run_id) = pr_test_app(Some("ghu_test"), None);

    create_run_with_pull_request_record(
        &state,
        run_id,
        "https://github.com/acme/widgets/pull/42",
        42,
        "Fix the bug",
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/pull_request/merge")))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "method": "bogus" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn merge_run_pull_request_returns_service_unavailable_without_github_credentials() {
    let (state, app, run_id) = pr_test_app(None, None);

    create_run_with_pull_request_record(
        &state,
        run_id,
        "https://github.com/acme/widgets/pull/42",
        42,
        "Fix the bug",
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/pull_request/merge")))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "method": "squash" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::SERVICE_UNAVAILABLE).await;

    assert_eq!(body["errors"][0]["code"], "integration_unavailable");
}

#[tokio::test]
async fn merge_run_pull_request_uses_stored_link_coordinates() {
    let github = MockServer::start();
    let github_mock = github.mock(|when, then| {
        when.method("PUT")
            .path("/repos/acme/widgets/pulls/42/merge")
            .header("authorization", "Bearer ghu_test")
            .json_body(json!({ "merge_method": "squash" }));
        then.status(200)
            .header("content-type", "application/json")
            .body(json!({}).to_string());
    });
    let (state, app, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));

    create_run_with_linked_pull_request_record(&state, run_id, PullRequestLink {
        owner:  "acme".to_string(),
        repo:   "widgets".to_string(),
        number: 42,
    })
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/pull_request/merge")))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "method": "squash" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;

    assert_eq!(body["number"], 42);
    assert_eq!(body["html_url"], "https://github.com/acme/widgets/pull/42");
    github_mock.assert();
}

#[tokio::test]
async fn close_run_pull_request_returns_not_found_when_record_missing() {
    let (_state, app, run_id) = pr_test_app_with_minimal_run(Some("ghu_test"), None).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/pull_request/close")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::NOT_FOUND).await;

    assert_eq!(body["errors"][0]["code"], "no_stored_record");
}

#[tokio::test]
async fn close_run_pull_request_returns_service_unavailable_without_github_credentials() {
    let (state, app, run_id) = pr_test_app(None, None);

    create_run_with_pull_request_record(
        &state,
        run_id,
        "https://github.com/acme/widgets/pull/42",
        42,
        "Fix the bug",
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/pull_request/close")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::SERVICE_UNAVAILABLE).await;

    assert_eq!(body["errors"][0]["code"], "integration_unavailable");
}

#[tokio::test]
async fn close_run_pull_request_returns_bad_gateway_when_github_pr_is_missing() {
    let github = MockServer::start();
    let github_mock = github.mock(|when, then| {
        when.method("PATCH")
            .path("/repos/acme/widgets/pulls/42")
            .header("authorization", "Bearer ghu_test");
        then.status(404)
            .header("content-type", "application/json")
            .body(json!({ "message": "Not Found" }).to_string());
    });
    let (state, app, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));

    create_run_with_pull_request_record(
        &state,
        run_id,
        "https://github.com/acme/widgets/pull/42",
        42,
        "Fix the bug",
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/pull_request/close")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::BAD_GATEWAY).await;

    assert_eq!(body["errors"][0]["code"], "github_not_found");
    github_mock.assert();
}

#[tokio::test]
async fn staleness_supervisor_updates_dirty_run_pull_request_branch() {
    let github = MockServer::start();
    let created_at = Utc::now().to_rfc3339();
    let _get_mock = dirty_run_pull_request_mock(&github, "open", false, "dirty", &created_at);
    let update_mock = update_branch_mock(&github, 202);
    let close_mock = close_branch_mock(&github);
    let (state, _app, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = super::pull_request_supervisor::StalePrState::default();
    super::pull_request_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    update_mock.assert();
    assert_eq!(
        close_mock.calls(),
        0,
        "a successful update must not close the PR"
    );
    assert!(
        counters.update_failures.is_empty(),
        "a successful update resets the failure counter"
    );
    assert!(
        run_pull_request_closed_events(&state, &run_id)
            .await
            .is_empty(),
        "no close event may be emitted on a successful update"
    );
}

#[tokio::test]
async fn staleness_supervisor_keeps_pr_open_on_conflict_and_counts_the_failure() {
    let github = MockServer::start();
    let created_at = Utc::now().to_rfc3339();
    let _get_mock = dirty_run_pull_request_mock(&github, "open", false, "dirty", &created_at);
    let update_mock = update_branch_mock(&github, 422);
    let close_mock = close_branch_mock(&github);
    // Mixed conflict (fabro-895d): the conflict intersection includes a code
    // file outside the loop's tracker paths, so the supervisor must strike.
    let _head_compare = compare_files_mock(&github, "main...fabro/run/42", &[
        ".seeds/issues.jsonl",
        "lib/foo.rs",
    ]);
    let _base_compare = compare_files_mock(&github, "fabro/run/42...main", &["lib/foo.rs"]);
    let push_mock = push_ref_mock(&github, "fabro/run/42", 200);
    let (state, _app, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = super::pull_request_supervisor::StalePrState::default();
    super::pull_request_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    update_mock.assert();
    assert_eq!(
        close_mock.calls(),
        0,
        "a 422 conflict must not close the PR"
    );
    assert_eq!(
        push_mock.calls(),
        0,
        "a mixed JSONL+code conflict must not be auto-resolved"
    );
    assert_eq!(
        counters.update_failures.get(&run_id),
        Some(&1),
        "the conflict counts as one failed attempt"
    );
    assert!(
        run_pull_request_closed_events(&state, &run_id)
            .await
            .is_empty(),
        "no close event may be emitted on conflict"
    );
    // The stored link stays: the wait endpoint keeps observing the open PR and
    // can only answer closed_unmerged/timeout, routing the conductor manual.
    let projection = state
        .stores
        .runs
        .load_run_projection(&run_id)
        .await
        .unwrap();
    assert!(
        projection
            .expect("run projection should load")
            .pull_request
            .is_some()
    );
}

#[tokio::test]
async fn staleness_supervisor_closes_pr_after_three_failed_updates() {
    let github = MockServer::start();
    let created_at = Utc::now().to_rfc3339();
    let _get_mock = dirty_run_pull_request_mock(&github, "open", false, "dirty", &created_at);
    let update_mock = update_branch_mock(&github, 422);
    let close_mock = close_branch_mock(&github);
    let (state, _app, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = super::pull_request_supervisor::StalePrState::default();
    counters.update_failures.insert(run_id, 3);
    super::pull_request_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    close_mock.assert();
    assert_eq!(
        update_mock.calls(),
        0,
        "a capped PR is closed without another update attempt"
    );
    let closed = run_pull_request_closed_events(&state, &run_id).await;
    assert_eq!(
        closed,
        vec![(
            "stale_base".to_string(),
            "https://github.com/acme/widgets/pull/42".to_string()
        )],
        "the close must be recorded as a stale_base run event"
    );
    assert!(
        !counters.update_failures.contains_key(&run_id),
        "retiring the PR clears its counter"
    );
    let projection = state
        .stores
        .runs
        .load_run_projection(&run_id)
        .await
        .unwrap();
    assert!(
        projection
            .expect("run projection should load")
            .pull_request
            .is_none(),
        "the closed PR must no longer be linked to the run"
    );
}

#[tokio::test]
async fn staleness_supervisor_closes_pr_older_than_24h() {
    let github = MockServer::start();
    let created_at = (Utc::now() - ChronoDuration::hours(25)).to_rfc3339();
    let _get_mock = dirty_run_pull_request_mock(&github, "open", false, "dirty", &created_at);
    // Over-age PRs carrying non-bookkeeping changes keep closing (fabro-895d).
    let _files_mock = pr_files_mock(&github, 42, &["lib/foo.rs"]);
    let update_mock = update_branch_mock(&github, 202);
    let close_mock = close_branch_mock(&github);
    let (state, _app, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = super::pull_request_supervisor::StalePrState::default();
    super::pull_request_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    close_mock.assert();
    assert_eq!(
        update_mock.calls(),
        0,
        "an over-age PR is closed without an update attempt"
    );
    let closed = run_pull_request_closed_events(&state, &run_id).await;
    assert_eq!(
        closed,
        vec![(
            "stale_base".to_string(),
            "https://github.com/acme/widgets/pull/42".to_string()
        )],
        "the age close must be recorded as a stale_base run event"
    );
}

#[tokio::test]
async fn staleness_supervisor_skips_clean_and_blocked_run_pull_requests() {
    let github = MockServer::start();
    let created_at = Utc::now().to_rfc3339();
    let _clean_mock = dirty_run_pull_request_mock(&github, "open", true, "clean", &created_at);
    let update_mock = update_branch_mock(&github, 202);
    let close_mock = close_branch_mock(&github);
    let (state, _app, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = super::pull_request_supervisor::StalePrState::default();
    counters.update_failures.insert(run_id, 2);
    super::pull_request_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    assert_eq!(
        update_mock.calls(),
        0,
        "a clean PR must not be update-branched"
    );
    assert_eq!(close_mock.calls(), 0, "a clean PR must not be closed");
    assert!(
        !counters.update_failures.contains_key(&run_id),
        "a clean PR resets its counter"
    );
}

#[tokio::test]
async fn staleness_supervisor_resolves_jsonl_only_conflict_without_strike() {
    let github = MockServer::start();
    let created_at = Utc::now().to_rfc3339();
    let _get_mock = dirty_run_pull_request_mock(&github, "open", false, "dirty", &created_at);
    let update_mock = update_branch_mock(&github, 422);
    let close_mock = close_branch_mock(&github);
    // Conflict intersection is the tracker JSONL only: the head also added a
    // fresh code file, but the base moved only `.seeds/issues.jsonl`.
    let _head_compare = compare_files_mock(&github, "main...fabro/run/42", &[
        ".seeds/issues.jsonl",
        "lib/new.rs",
    ]);
    let _base_compare =
        compare_files_mock(&github, "fabro/run/42...main", &[".seeds/issues.jsonl"]);
    let base_tracker = concat!(
        "{\"id\":\"fabro-1\",\"status\":\"closed\"}\n",
        "{\"id\":\"fabro-2\",\"status\":\"open\"}\n",
    );
    let head_tracker = concat!(
        // stale pre-fork copy that base has since closed
        "{\"id\":\"fabro-1\",\"status\":\"open\"}\n",
        // branch-side update of an open record
        "{\"id\":\"fabro-2\",\"status\":\"in_progress\"}\n",
        // branch-only record
        "{\"id\":\"fabro-9\",\"status\":\"open\"}\n",
    );
    let _base_contents = raw_contents_mock(&github, ".seeds/issues.jsonl", "main", base_tracker);
    let _head_contents =
        raw_contents_mock(&github, ".seeds/issues.jsonl", "fabro/run/42", head_tracker);
    // Diff-based publish (fabro-4ebd): the run's own non-tracker change is
    // carried onto the merge tree from the head side, by content.
    let _head_code_contents =
        raw_contents_mock(&github, "lib/new.rs", "fabro/run/42", "fn new() {}\n");
    let _head_commit = commit_info_mock(&github, "fabro/run/42", "h1", "th");
    let _base_commit = commit_info_mock(&github, "main", "b1", "tb");
    // The stored blob must carry the closed-wins union. The request body is
    // JSON-escaped, so the matchers use the escaped form: base-closed fabro-1
    // stays closed, the head's in_progress update wins, and the branch-only
    // fabro-9 record survives.
    let blob_mock = github.mock(|when, then| {
        when.method("POST")
            .path("/repos/acme/widgets/git/blobs")
            .header("authorization", "Bearer ghu_test")
            .body_includes("fabro-1\\\",\\\"status\\\":\\\"closed")
            .body_includes("fabro-2\\\",\\\"status\\\":\\\"in_progress")
            .body_includes("fabro-9");
        then.status(201).json_body(json!({ "sha": "blob1" }));
    });
    let _code_blob_mock = github.mock(|when, then| {
        when.method("POST")
            .path("/repos/acme/widgets/git/blobs")
            .header("authorization", "Bearer ghu_test")
            .body_includes("fn new() {}");
        then.status(201).json_body(json!({ "sha": "blob-code" }));
    });
    // The merge tree must grow from the CURRENT base tree ("tb"), not the
    // run's (possibly stale) head tree — the base's concurrent work survives
    // verbatim (fabro-4ebd (b)).
    let _tree_mock = github.mock(|when, then| {
        when.method("POST")
            .path("/repos/acme/widgets/git/trees")
            .header("authorization", "Bearer ghu_test")
            .body_includes("\"base_tree\":\"tb\"");
        then.status(201).json_body(json!({ "sha": "t2" }));
    });
    let _merge_commit_mock = git_object_create_mock(&github, "git/commits", "m1");
    let push_mock = push_ref_mock(&github, "fabro/run/42", 200);
    let (state, _app, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = super::pull_request_supervisor::StalePrState::default();
    super::pull_request_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    update_mock.assert();
    blob_mock.assert();
    push_mock.assert();
    assert_eq!(
        close_mock.calls(),
        0,
        "a resolved JSONL conflict must not close the PR"
    );
    assert!(
        counters.update_failures.is_empty(),
        "a resolved JSONL conflict resets the strike counter"
    );
    assert!(
        counters.parked.is_empty(),
        "a resolved JSONL conflict must not park the PR"
    );
    assert!(
        run_pull_request_closed_events(&state, &run_id)
            .await
            .is_empty(),
        "no close event may be emitted on a resolved conflict"
    );
}

#[tokio::test]
async fn staleness_supervisor_parks_pr_when_jsonl_resolution_fails() {
    let github = MockServer::start();
    let created_at = Utc::now().to_rfc3339();
    let _get_mock = dirty_run_pull_request_mock(&github, "open", false, "dirty", &created_at);
    let update_mock = update_branch_mock(&github, 422);
    let close_mock = close_branch_mock(&github);
    let _head_compare =
        compare_files_mock(&github, "main...fabro/run/42", &[".seeds/issues.jsonl"]);
    let _base_compare =
        compare_files_mock(&github, "fabro/run/42...main", &[".seeds/issues.jsonl"]);
    let _base_contents = raw_contents_mock(
        &github,
        ".seeds/issues.jsonl",
        "main",
        "{\"id\":\"fabro-1\",\"status\":\"open\"}\n",
    );
    // Unparseable head-side tracker state: the union cannot be computed.
    let _head_contents = raw_contents_mock(
        &github,
        ".seeds/issues.jsonl",
        "fabro/run/42",
        "not jsonl at all\n",
    );
    let blob_mock = git_object_create_mock(&github, "git/blobs", "blob1");
    let push_mock = push_ref_mock(&github, "fabro/run/42", 200);
    let (state, _app, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = super::pull_request_supervisor::StalePrState::default();
    super::pull_request_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    assert_eq!(blob_mock.calls(), 0, "no merge state may be written");
    assert_eq!(push_mock.calls(), 0, "no push may be attempted");
    assert_eq!(
        close_mock.calls(),
        0,
        "a failed JSONL resolution must park, not close"
    );
    assert!(
        counters.parked.contains_key(&run_id),
        "the PR must be parked after a failed resolution"
    );
    assert!(
        !counters.update_failures.contains_key(&run_id),
        "parking does not count a strike"
    );
    assert!(
        run_pull_request_closed_events(&state, &run_id)
            .await
            .is_empty(),
        "no close event may be emitted when parking"
    );

    // A parked PR receives no further update attempts on later passes.
    let update_calls = update_mock.calls();
    super::pull_request_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();
    assert_eq!(
        update_mock.calls(),
        update_calls,
        "a parked PR is skipped entirely on the next pass"
    );
}

#[tokio::test]
async fn staleness_supervisor_parks_over_age_bookkeeping_only_pr() {
    let github = MockServer::start();
    let created_at = (Utc::now() - ChronoDuration::hours(25)).to_rfc3339();
    let _get_mock = dirty_run_pull_request_mock(&github, "open", false, "dirty", &created_at);
    // Over-age PR whose changed paths are all dev-loop bookkeeping
    // (tracker files plus stage journals) parks instead of closing
    // (fabro-895d).
    let _files_mock = pr_files_mock(&github, 42, &[
        ".seeds/issues.jsonl",
        ".mulch/mulch.config.yaml",
        ".fabro/journal/run.jsonl",
    ]);
    let update_mock = update_branch_mock(&github, 202);
    let close_mock = close_branch_mock(&github);
    let (state, _app, run_id) = pr_test_app(Some("ghu_test"), Some(github.base_url()));
    staleness_test_run(&state, run_id).await;

    let mut counters = super::pull_request_supervisor::StalePrState::default();
    super::pull_request_supervisor::process_stale_pull_requests(&state, &mut counters)
        .await
        .unwrap();

    assert_eq!(
        close_mock.calls(),
        0,
        "an over-age bookkeeping-only PR must park, not close"
    );
    assert_eq!(
        update_mock.calls(),
        0,
        "an over-age PR is not update-branched"
    );
    assert!(
        counters.parked.contains_key(&run_id),
        "the over-age bookkeeping-only PR must be parked"
    );
    assert!(
        run_pull_request_closed_events(&state, &run_id)
            .await
            .is_empty(),
        "no close event may be emitted when parking"
    );
    let projection = state
        .stores
        .runs
        .load_run_projection(&run_id)
        .await
        .unwrap();
    assert!(
        projection
            .expect("run projection should load")
            .pull_request
            .is_some(),
        "the parked PR stays linked to the run"
    );
}

/// fabro-06e0: the live pull request details route is a run-management
/// read, so an inspects-declared worker (develop planner, revisor) can
/// enrich linked PR state for runs of workflows it inspects — without
/// the worker ever holding GitHub credentials.
#[tokio::test]
async fn inspects_worker_reads_live_pull_request_details_for_declared_workflow_runs() {
    let (state, app) = jwt_auth_app();
    let parent_run_id = unique_run_id();
    let develop_run_id = unique_run_id();
    let nightly_run_id = unique_run_id();
    let develop_run_store = create_slack_notification_run(
        &state,
        develop_run_id,
        fabro_types::WorkflowSettings::default(),
        "develop",
        Some("develop"),
    )
    .await;
    create_run_with_workflow_slug(&state, nightly_run_id, "nightly").await;
    workflow_event::append_event(
        &develop_run_store,
        &develop_run_id,
        &workflow_event::Event::PullRequestLinked {
            pull_request: PullRequestLink {
                owner:  "fabro-sh".to_string(),
                repo:   "fabro".to_string(),
                number: 47,
            },
        },
    )
    .await
    .unwrap();
    let inspects_token = issue_test_inspects_worker_token(&parent_run_id, &["develop".to_string()]);
    let plain_run_tools_token = issue_test_run_tools_worker_token(&parent_run_id);

    // Declared workflow: the read reaches the handler and returns the
    // stored link (details degrade to unavailable — no GitHub creds on
    // this test app — proving the authz, not the integration, is the
    // subject here).
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{develop_run_id}/pull_request"),
            &inspects_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["data"]["link"]["number"], 47);
    assert_eq!(body["meta"]["details_status"], "unavailable");

    // Foreign workflow: rejected even though the token is otherwise valid.
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{nightly_run_id}/pull_request"),
            &inspects_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    // Run-tools workers without inspects authority keep being rejected.
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{develop_run_id}/pull_request"),
            &plain_run_tools_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;
}
