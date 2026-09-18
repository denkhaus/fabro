use super::*;

#[tokio::test]
async fn create_run_response_includes_web_url_when_web_enabled() {
    let state = test_app_state_with_options(
        server_settings_from_toml(
            r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
enabled = true
url = "http://127.0.0.1:32276"
"#,
        ),
        RunLayer::default(),
        5,
    );
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api("/runs"))
                .header("content-type", "application/json")
                .body(intent_body(&app, MINIMAL_DOT).await)
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::CREATED).await;
    let id = body["id"].as_str().expect("id should be a string");
    assert_eq!(
        body["links"]["web"].as_str(),
        Some(format!("http://127.0.0.1:32276/runs/{id}").as_str()),
    );
}

#[tokio::test]
async fn create_run_response_omits_web_url_when_web_disabled() {
    let state = test_app_state_with_options(
        server_settings_from_toml(
            r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
enabled = false
url = "http://127.0.0.1:32276"
"#,
        ),
        RunLayer::default(),
        5,
    );
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api("/runs"))
                .header("content-type", "application/json")
                .body(intent_body(&app, MINIMAL_DOT).await)
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::CREATED).await;
    assert!(
        body.get("web_url").is_none() || body["web_url"].is_null(),
        "web_url should be absent or null when web is disabled, got {body}"
    );
}

#[tokio::test]
async fn create_run_without_explicit_title_returns_deterministic_then_updates_generated_title() {
    let llm = MockServer::start_async().await;
    let title_mock = mock_openai_title_response(&llm, "Generated deploy title", None).await;
    let state = TestAppStateBuilder::new()
        .provider_base_url("openai", llm.url("/v1"))
        .env_lookup(|_| None)
        .build();
    state
        .stores
        .vault
        .set("OPENAI_API_KEY", "openai-key", SecretType::Token, None)
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let body = post_run_intent(&app, test_intent(&app, MINIMAL_DOT).await).await;
    let run_id: RunId = body["id"].as_str().unwrap().parse().unwrap();

    assert_eq!(body["title"], "Test");
    wait_for_run_title(&state, run_id, "Generated deploy title").await;
    assert_eq!(title_update_event_count(&state, run_id).await, 1);
    title_mock.assert_async().await;
}

#[tokio::test]
async fn create_run_with_explicit_title_skips_generated_title_work() {
    let llm = MockServer::start_async().await;
    let title_mock = mock_openai_title_response(&llm, "Generated deploy title", None).await;
    let state = TestAppStateBuilder::new()
        .provider_base_url("openai", llm.url("/v1"))
        .env_lookup(|_| None)
        .build();
    state
        .stores
        .vault
        .set("OPENAI_API_KEY", "openai-key", SecretType::Token, None)
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let mut intent = test_intent(&app, MINIMAL_DOT).await;
    intent["title"] = json!("Caller title");

    let body = post_run_intent(&app, intent).await;
    let run_id: RunId = body["id"].as_str().unwrap().parse().unwrap();
    // The spawn gate is synchronous in `create_run`, so once the response
    // returns we know no title task was scheduled. No sleep needed.

    assert_eq!(
        state
            .stores
            .run_summaries
            .get(&run_id, Utc::now())
            .await
            .unwrap()
            .unwrap()
            .title,
        "Caller title"
    );
    assert_eq!(title_update_event_count(&state, run_id).await, 0);
    title_mock.assert_calls_async(0).await;
}

#[tokio::test]
async fn generated_title_failure_leaves_deterministic_title_unchanged() {
    let llm = MockServer::start_async().await;
    let title_mock = llm
        .mock_async(|when, then| {
            when.method(POST).path("/v1/responses");
            then.status(500)
                .header("content-type", "application/json")
                .json_body(json!({"error": {"message": "boom"}}));
        })
        .await;
    let state = TestAppStateBuilder::new()
        .provider_base_url("openai", llm.url("/v1"))
        .env_lookup(|_| None)
        .build();
    state
        .stores
        .vault
        .set("OPENAI_API_KEY", "openai-key", SecretType::Token, None)
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let body = post_run_intent(&app, test_intent(&app, MINIMAL_DOT).await).await;
    let run_id: RunId = body["id"].as_str().unwrap().parse().unwrap();
    wait_for_mock_hits(&title_mock, 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;

    assert_eq!(
        state
            .stores
            .run_summaries
            .get(&run_id, Utc::now())
            .await
            .unwrap()
            .unwrap()
            .title,
        "Test"
    );
    assert_eq!(title_update_event_count(&state, run_id).await, 0);
}

#[tokio::test]
async fn generated_title_does_not_overwrite_user_title_edit() {
    let llm = MockServer::start_async().await;
    let title_mock = mock_openai_title_response(
        &llm,
        "Generated deploy title",
        Some(std::time::Duration::from_millis(150)),
    )
    .await;
    let state = TestAppStateBuilder::new()
        .provider_base_url("openai", llm.url("/v1"))
        .env_lookup(|_| None)
        .build();
    state
        .stores
        .vault
        .set("OPENAI_API_KEY", "openai-key", SecretType::Token, None)
        .await
        .unwrap();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let body = post_run_intent(&app, test_intent(&app, MINIMAL_DOT).await).await;
    let run_id: RunId = body["id"].as_str().unwrap().parse().unwrap();
    let patch = Request::builder()
        .method("PATCH")
        .uri(api(&format!("/runs/{run_id}")))
        .header("content-type", "application/json")
        .body(Body::from(json!({"title": "User title"}).to_string()))
        .unwrap();
    let response = app.clone().oneshot(patch).await.unwrap();
    response_json!(response, StatusCode::OK).await;

    wait_for_mock_hits(&title_mock, 1).await;
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;

    assert_eq!(
        state
            .stores
            .run_summaries
            .get(&run_id, Utc::now())
            .await
            .unwrap()
            .unwrap()
            .title,
        "User title"
    );
    assert_eq!(title_update_event_count(&state, run_id).await, 1);
}

#[tokio::test]
async fn post_runs_run_intent_derives_workflow_slug_from_immutable_entrypoint() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    for (entrypoint, expected_slug) in [
        ("deploy/workflow.fabro", "deploy"),
        ("workflow.fabro", "workflow"),
    ] {
        let workflow_version_id =
            store_workflow_version_with_entrypoint(&state, entrypoint, MINIMAL_DOT, None).await;
        let body = post_run_intent(
            &app,
            json!({
                "workflow_version_id": workflow_version_id,
                "target": { "kind": "none" },
                "args": {}
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
            projection.spec.workflow_slug.as_deref(),
            Some(expected_slug)
        );
        assert_eq!(
            projection.spec.workflow_version_id,
            Some(workflow_version_id)
        );
    }
}

#[tokio::test]
async fn post_runs_run_intent_persists_tagged_exact_git_target_without_starting() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let workflow_version_id = store_workflow_version(
        &state,
        MINIMAL_DOT,
        Some(
            r#"
_version = 1

[environments.default]
provider = "local"

[environments.default.image]
dockerfile = { path = "Dockerfile" }

[environments.default.resources]
cpu = 7

[environments.default.env]
WORKFLOW_OVERLAY = "present"

[run.goal]
file = "goal.md"

[run.environment.image]
docker = "workflow-owned:latest"
"#,
        ),
    )
    .await;
    let submitted_sha = "ABCDEF0123456789ABCDEF0123456789ABCDEF01";
    let body = post_run_intent(
        &app,
        json!({
            "workflow_version_id": workflow_version_id,
            "target": {
                "kind": "git",
                "repo": "fabro-sh/fabro",
                "branch": "feature/run-intent",
                "tag": "v1.2.3",
                "sha": submitted_sha
            },
            "args": {
                "inputs": { "ship": true },
                "labels": { "team": "platform" }
            },
            "title": "Intent run"
        }),
    )
    .await;
    let run_id = body["id"].as_str().unwrap().parse::<RunId>().unwrap();

    assert_eq!(body["lifecycle"]["status"]["kind"], "submitted");
    let run_store = state.stores.runs.open_run_reader(&run_id).await.unwrap();
    let events = run_store.list_events().await.unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event.event.event_name())
            .collect::<Vec<_>>(),
        vec!["run.created", "run.submitted"]
    );
    let projection = run_store.state().await.unwrap();
    assert_eq!(
        projection.spec.workflow_version_id,
        Some(workflow_version_id)
    );
    assert_eq!(
        projection.spec.graph.goal(),
        "Goal loaded from immutable version bytes"
    );
    assert_eq!(
        projection.spec.target,
        Some(fabro_types::RunTarget::Git(fabro_types::GitRunTarget {
            repo:   "fabro-sh/fabro".to_string(),
            branch: "feature/run-intent".to_string(),
            tag:    Some("v1.2.3".to_string()),
            sha:    Some("abcdef0123456789abcdef0123456789abcdef01".to_string()),
        }))
    );
    assert_eq!(
        projection
            .spec
            .git
            .as_ref()
            .and_then(|git| git.sha.as_deref()),
        Some("abcdef0123456789abcdef0123456789abcdef01")
    );
    assert_eq!(
        projection.spec.settings.run.inputs["ship"],
        toml::Value::Boolean(true)
    );
    assert_eq!(
        projection.spec.labels.get("team").map(String::as_str),
        Some("platform")
    );
    assert_eq!(
        projection.spec.settings.run.environment.provider,
        SandboxProviderKind::DOCKER
    );
    assert_eq!(
        projection
            .spec
            .settings
            .run
            .environment
            .image
            .docker
            .as_deref(),
        Some("buildpack-deps:noble")
    );
    assert!(
        projection
            .spec
            .settings
            .run
            .environment
            .image
            .dockerfile
            .is_none()
    );
    assert_eq!(
        projection.spec.settings.run.environment.resources.cpu,
        Some(7)
    );
    assert!(
        projection
            .spec
            .settings
            .run
            .environment
            .env
            .contains_key("WORKFLOW_OVERLAY")
    );
}

#[tokio::test]
async fn post_runs_run_intent_creates_submitted_none_target_without_git_projection() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;
    let body = post_run_intent(
        &app,
        json!({
            "workflow_version_id": workflow_version_id,
            "target": { "kind": "none" },
            "args": {}
        }),
    )
    .await;
    let run_id = body["id"].as_str().unwrap().parse::<RunId>().unwrap();

    assert_eq!(body["lifecycle"]["status"]["kind"], "submitted");
    let run_store = state.stores.runs.open_run_reader(&run_id).await.unwrap();
    let events = run_store.list_events().await.unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event.event.event_name())
            .collect::<Vec<_>>(),
        vec!["run.created", "run.submitted"]
    );
    let projection = run_store.state().await.unwrap();
    assert_eq!(
        projection.spec.target,
        Some(fabro_types::RunTarget::None {})
    );
    assert_eq!(
        projection.spec.workflow_version_id,
        Some(workflow_version_id)
    );
    assert_eq!(projection.spec.source_directory, None);
    assert_eq!(projection.spec.git, None);
    assert!(projection.spec.settings.run.clone.enabled);
    assert!(projection.spec.definition_blob.is_some());
}

#[tokio::test]
async fn post_runs_run_intent_args_true_override_resolved_settings_without_starting() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let state = TestAppStateBuilder::new()
        .default_environment_provider(Some(SandboxProviderKind::LOCAL))
        .env_lookup(|_| None)
        .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
        .build();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;
    let body = post_run_intent(
        &app,
        json!({
            "workflow_version_id": workflow_version_id,
            "target": { "kind": "folder", "path": &workspace },
            "args": {
                "dry_run": true,
                "auto_approve": true,
                "preserve_sandbox": true
            }
        }),
    )
    .await;
    let run_id = body["id"].as_str().unwrap().parse::<RunId>().unwrap();

    assert_eq!(body["lifecycle"]["status"]["kind"], "submitted");
    let run_store = state.stores.runs.open_run_reader(&run_id).await.unwrap();
    let projection = run_store.state().await.unwrap();
    assert_eq!(projection.spec.settings.run.execution.mode, RunMode::DryRun);
    assert_eq!(
        projection.spec.settings.run.execution.approval,
        ApprovalMode::Auto
    );
    assert!(projection.spec.settings.run.environment.lifecycle.preserve);
}

#[tokio::test]
async fn post_runs_run_intent_dry_run_rejects_configured_target_mismatches() {
    let states_and_targets = [
        (
            local_test_app_state(),
            json!({
                "kind": "git",
                "repo": "fabro-sh/fabro",
                "branch": "main"
            }),
        ),
        (local_test_app_state(), json!({ "kind": "none" })),
        (
            test_app_state(),
            json!({ "kind": "folder", "path": "/path-that-must-not-be-read" }),
        ),
        (
            TestAppStateBuilder::new()
                .default_environment_provider(Some(SandboxProviderKind::DAYTONA))
                .vault_entries([(fabro_static::EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
                .build(),
            json!({ "kind": "folder", "path": "/path-that-must-not-be-read" }),
        ),
    ];

    for (state, target) in states_and_targets {
        let app = crate::test_support::build_test_router(Arc::clone(&state));
        let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;
        let response = post_run_intent_response(
            &app,
            json!({
                "workflow_version_id": workflow_version_id,
                "target": target,
                "args": { "dry_run": true }
            }),
        )
        .await;
        let body = response_json!(response, StatusCode::UNPROCESSABLE_ENTITY).await;

        assert_eq!(body["errors"][0]["code"], "target_environment_unsupported");
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_runs_run_intent_dry_run_starts_in_isolated_scratch_workspace() {
    let source = r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[[run.prepare.steps]]
script = "pwd > setup-working-directory.txt"
"#;
    let state = test_app_state_with_settings_and_registry_factory(
        server_settings_from_toml(source),
        manifest_run_defaults_from_toml(source),
        |interviewer| fabro_workflow::handler::default_registry(interviewer, || None),
    );
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let external_target = tempfile::tempdir().unwrap();
    let external_sentinel = external_target.path().join("existing-target-file.txt");
    tokio::fs::write(&external_sentinel, b"must remain unchanged")
        .await
        .unwrap();
    let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;
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

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/start")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    response_json!(response, StatusCode::OK).await;

    execute_run(Arc::clone(&state), run_id).await;

    let run_store = state.stores.runs.open_run_reader(&run_id).await.unwrap();
    assert_eq!(
        run_store.state().await.unwrap().status,
        RunStatus::Succeeded {
            reason: SuccessReason::Completed,
        }
    );
    let scratch_workspace = Storage::new(state.server_storage_dir())
        .run_scratch(&run_id)
        .root()
        .join("dry-run-workspace")
        .canonicalize()
        .unwrap();
    let setup_working_directory =
        tokio::fs::read_to_string(scratch_workspace.join("setup-working-directory.txt"))
            .await
            .unwrap();
    assert_eq!(Path::new(setup_working_directory.trim()), scratch_workspace);
    assert_eq!(
        tokio::fs::read(&external_sentinel).await.unwrap(),
        b"must remain unchanged"
    );
    assert!(
        !external_target
            .path()
            .join("setup-working-directory.txt")
            .exists()
    );
}

#[tokio::test]
async fn post_runs_run_intent_args_false_are_distinct_from_omitted_overrides() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();
    let state = TestAppStateBuilder::new()
        .runtime_settings(
            default_test_server_settings(),
            manifest_run_defaults_from_toml(
                r#"
[run.execution]
mode = "dry_run"
approval = "auto"

[run.environment.lifecycle]
preserve = true
"#,
            ),
        )
        .default_environment_provider(Some(SandboxProviderKind::LOCAL))
        .env_lookup(|_| None)
        .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
        .build();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;

    let explicit_false = post_run_intent(
        &app,
        json!({
            "workflow_version_id": workflow_version_id,
            "target": { "kind": "folder", "path": &workspace },
            "args": {
                "dry_run": false,
                "auto_approve": false,
                "preserve_sandbox": false
            }
        }),
    )
    .await;
    let omitted = post_run_intent(
        &app,
        json!({
            "workflow_version_id": workflow_version_id,
            "target": { "kind": "folder", "path": &workspace },
            "args": {}
        }),
    )
    .await;

    assert_eq!(explicit_false["lifecycle"]["status"]["kind"], "submitted");
    assert_eq!(omitted["lifecycle"]["status"]["kind"], "submitted");
    let explicit_false_id = explicit_false["id"]
        .as_str()
        .unwrap()
        .parse::<RunId>()
        .unwrap();
    let omitted_id = omitted["id"].as_str().unwrap().parse::<RunId>().unwrap();
    let explicit_false_store = state
        .stores
        .runs
        .open_run_reader(&explicit_false_id)
        .await
        .unwrap();
    let explicit_false = explicit_false_store.state().await.unwrap();
    let omitted_store = state
        .stores
        .runs
        .open_run_reader(&omitted_id)
        .await
        .unwrap();
    let omitted = omitted_store.state().await.unwrap();

    assert_eq!(
        explicit_false.spec.settings.run.execution.mode,
        RunMode::Normal
    );
    assert_eq!(
        explicit_false.spec.settings.run.execution.approval,
        ApprovalMode::Prompt
    );
    assert!(
        !explicit_false
            .spec
            .settings
            .run
            .environment
            .lifecycle
            .preserve
    );
    assert_eq!(omitted.spec.settings.run.execution.mode, RunMode::DryRun);
    assert_eq!(
        omitted.spec.settings.run.execution.approval,
        ApprovalMode::Auto
    );
    assert!(omitted.spec.settings.run.environment.lifecycle.preserve);
}

#[tokio::test]
async fn post_runs_run_intent_canonicalizes_and_persists_a_local_folder_target() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    let hop = dir.path().join("hop");
    std::fs::create_dir(&workspace).unwrap();
    std::fs::create_dir(&hop).unwrap();
    // Target-project files are not compiler inputs for a version-backed run.
    std::fs::write(workspace.join("workflow.toml"), "not valid TOML").unwrap();
    std::fs::write(workspace.join("goal.md"), "Goal from target folder").unwrap();
    let submitted = hop.join("..").join("workspace");
    let canonical = workspace
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let state = local_test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let workflow_version_id = store_workflow_version(
        &state,
        MINIMAL_DOT,
        Some("_version = 1\n[run.goal]\nfile = \"goal.md\"\n"),
    )
    .await;

    let body = post_run_intent(
        &app,
        folder_intent(workflow_version_id, submitted.to_string_lossy()),
    )
    .await;
    let run_id = body["id"].as_str().unwrap().parse::<RunId>().unwrap();

    assert_eq!(body["lifecycle"]["status"]["kind"], "submitted");
    let run_store = state.stores.runs.open_run_reader(&run_id).await.unwrap();
    let events = run_store.list_events().await.unwrap();
    assert_eq!(
        events
            .iter()
            .map(|event| event.event.event_name())
            .collect::<Vec<_>>(),
        vec!["run.created", "run.submitted"]
    );
    let projection = run_store.state().await.unwrap();
    assert_eq!(
        projection.spec.target,
        Some(fabro_types::RunTarget::Folder {
            path: canonical.clone(),
        })
    );
    assert_eq!(
        projection.spec.source_directory.as_deref(),
        Some(canonical.as_str())
    );
    assert_eq!(projection.spec.git, None);
    assert_eq!(
        projection.spec.graph.goal(),
        "Goal loaded from immutable version bytes"
    );
    assert_eq!(
        projection.spec.settings.run.environment.provider,
        SandboxProviderKind::LOCAL
    );
    assert!(projection.spec.definition_blob.is_some());
}

#[tokio::test]
async fn post_runs_run_intent_observes_folder_git_metadata_without_a_remote_call() {
    let dir = tempfile::tempdir().unwrap();
    let repo = git2::Repository::init(dir.path()).unwrap();
    let mut index = repo.index().unwrap();
    let tree_id = index.write_tree().unwrap();
    drop(index);
    let tree = repo.find_tree(tree_id).unwrap();
    let signature = git2::Signature::now("Fabro Test", "fabro@example.com").unwrap();
    let commit = repo
        .commit(Some("HEAD"), &signature, &signature, "initial", &tree, &[])
        .unwrap();
    let commit = commit.to_string();
    drop(tree);
    repo.remote("origin", "https://github.com/acme/widgets.git")
        .unwrap();
    drop(repo);
    let canonical = dir
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let state = local_test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;

    let body = post_run_intent(&app, folder_intent(workflow_version_id, canonical)).await;
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
    let git = projection.spec.git.unwrap();

    assert_eq!(git.origin_url, "https://github.com/acme/widgets");
    assert!(!git.branch.is_empty());
    assert_eq!(git.sha.as_deref(), Some(commit.as_str()));
    assert_eq!(git.dirty, fabro_types::DirtyStatus::Clean);
}

#[tokio::test]
async fn post_runs_run_intent_rejects_invalid_folder_paths_before_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file");
    std::fs::write(&file, "not a directory").unwrap();
    let state = local_test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;
    let invalid_paths = [
        String::new(),
        "relative/path".to_string(),
        dir.path().join("missing").to_string_lossy().to_string(),
        file.to_string_lossy().to_string(),
    ];

    for path in invalid_paths {
        let response =
            post_run_intent_response(&app, folder_intent(workflow_version_id, path)).await;
        let body = response_json!(response, StatusCode::UNPROCESSABLE_ENTITY).await;
        assert_eq!(body["errors"][0]["code"], "target_invalid");
    }

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
async fn post_runs_run_intent_applies_the_folder_target_environment_matrix() {
    let dir = tempfile::tempdir().unwrap();
    // A missing path proves provider admission wins over filesystem
    // materialization. Touching the path first would return `target_invalid`
    // instead of the provider-specific errors asserted below.
    let target = dir.path().join("missing").to_string_lossy().to_string();

    for state in [
        test_app_state(),
        TestAppStateBuilder::new()
            .default_environment_provider(Some(SandboxProviderKind::DAYTONA))
            .vault_entries([(fabro_static::EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
            .build(),
    ] {
        let app = crate::test_support::build_test_router(Arc::clone(&state));
        let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;
        let response =
            post_run_intent_response(&app, folder_intent(workflow_version_id, &target)).await;
        let body = response_json!(response, StatusCode::UNPROCESSABLE_ENTITY).await;
        assert_eq!(body["errors"][0]["code"], "target_environment_unsupported");
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

    let disabled_state = TestAppStateBuilder::new()
        .runtime_settings(
            server_settings_from_toml(
                r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.sandbox.providers.local]
enabled = false
"#,
            ),
            RunLayer::default(),
        )
        .default_environment_provider(Some(SandboxProviderKind::LOCAL))
        .vault_entries([(fabro_static::EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
        .build();
    let app = crate::test_support::build_test_router(Arc::clone(&disabled_state));
    let workflow_version_id = store_workflow_version(&disabled_state, MINIMAL_DOT, None).await;
    let response =
        post_run_intent_response(&app, folder_intent(workflow_version_id, &target)).await;
    let body = response_json!(response, StatusCode::SERVICE_UNAVAILABLE).await;
    assert_eq!(body["errors"][0]["code"], "integration_unavailable");
    assert!(
        disabled_state
            .stores
            .run_summaries
            .list_identities()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn run_tools_worker_cannot_select_server_folder_from_clone_based_parent() {
    let dir = tempfile::tempdir().unwrap();
    let missing_target = dir.path().join("missing");
    let (state, app) = jwt_auth_app();
    let user_token = issue_test_user_jwt();
    let parent_run_id = create_run_with_bearer(&app, &user_token).await;
    let worker_token = issue_test_run_tools_worker_token(&parent_run_id);
    let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;
    let mut intent = folder_intent(workflow_version_id, missing_target.to_string_lossy());
    intent["environment_id"] = json!("local");
    intent["parent_id"] = json!(parent_run_id);

    let response = app
        .oneshot(json_bearer_request(
            Method::POST,
            "/runs",
            &worker_token,
            &intent,
        ))
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::UNPROCESSABLE_ENTITY).await;

    assert_eq!(body["errors"][0]["code"], "target_environment_unsupported");
    assert_eq!(
        body["errors"][0]["detail"],
        "folder targets created by a worker require a Local parent environment"
    );
    assert_eq!(
        state
            .stores
            .run_summaries
            .list_identities()
            .await
            .unwrap()
            .len(),
        1,
        "the rejected child must not be persisted"
    );
}

#[tokio::test]
async fn run_tools_worker_folder_target_from_missing_parent_run_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let (state, app) = jwt_auth_app();
    let worker_token = issue_test_run_tools_worker_token(&RunId::new());
    let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;
    let mut intent = folder_intent(workflow_version_id, dir.path().to_string_lossy());
    intent["environment_id"] = json!("local");

    let response = app
        .oneshot(json_bearer_request(
            Method::POST,
            "/runs",
            &worker_token,
            &intent,
        ))
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::NOT_FOUND).await;

    assert_eq!(body["errors"][0]["code"], "worker_run_not_found");
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
async fn run_tools_worker_can_select_server_folder_from_local_parent() {
    let dir = tempfile::tempdir().unwrap();
    let (state, app) = jwt_auth_app();
    let user_token = issue_test_user_jwt();
    let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;
    let mut parent_intent = folder_intent(workflow_version_id, dir.path().to_string_lossy());
    parent_intent["environment_id"] = json!("local");

    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            "/runs",
            &user_token,
            &parent_intent,
        ))
        .await
        .unwrap();
    let parent = response_json!(response, StatusCode::CREATED).await;
    let parent_run_id = parent["id"].as_str().unwrap().parse::<RunId>().unwrap();
    let worker_token = issue_test_run_tools_worker_token(&parent_run_id);
    let mut child_intent = folder_intent(workflow_version_id, dir.path().to_string_lossy());
    child_intent["environment_id"] = json!("local");
    child_intent["parent_id"] = json!(parent_run_id);

    let response = app
        .oneshot(json_bearer_request(
            Method::POST,
            "/runs",
            &worker_token,
            &child_intent,
        ))
        .await
        .unwrap();
    let child = response_json!(response, StatusCode::CREATED).await;

    assert_eq!(child["parent_id"], parent_run_id.to_string());
    assert_eq!(child["lifecycle"]["status"]["kind"], "submitted");
}

#[tokio::test]
async fn post_runs_run_intent_accepts_none_target_with_ready_daytona_environment() {
    let state = TestAppStateBuilder::new()
        .default_environment_provider(Some(SandboxProviderKind::DAYTONA))
        .vault_entries([
            (fabro_static::EnvVars::OPENAI_API_KEY, "test-openai-api-key"),
            (
                fabro_static::EnvVars::DAYTONA_API_KEY,
                "test-daytona-api-key",
            ),
        ])
        .build();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;
    let body = post_run_intent(
        &app,
        json!({
            "workflow_version_id": workflow_version_id,
            "target": { "kind": "none" },
            "args": {}
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
        projection.spec.target,
        Some(fabro_types::RunTarget::None {})
    );
    assert_eq!(
        projection.spec.settings.run.environment.provider,
        SandboxProviderKind::DAYTONA
    );
    assert_eq!(projection.spec.source_directory, None);
    assert_eq!(projection.spec.git, None);
}

#[tokio::test]
async fn post_runs_reports_malformed_json() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let malformed = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api("/runs"))
                .header("content-type", "application/json")
                .body(Body::from("{"))
                .unwrap(),
        )
        .await
        .unwrap();
    let malformed = response_json!(malformed, StatusCode::BAD_REQUEST).await;
    assert_eq!(malformed["errors"][0]["code"], "invalid_json");
}

#[tokio::test]
async fn post_runs_attributes_parse_failures_and_rejects_duplicate_keys() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let id = fabro_types::test_support::test_workflow_version_id();
    for (raw, expected_detail) in [
        ("{}".to_string(), "missing field"),
        (
            format!(
                r#"{{"workflow_version_id":"{id}","target":{{"kind":"none"}},"unexpected":true,"args":{{}}}}"#
            ),
            "unknown field",
        ),
        (
            format!(
                r#"{{"workflow_version_id":"{id}","workflow_version_id":"{id}","target":{{"kind":"none"}},"args":{{}}}}"#
            ),
            "duplicate field",
        ),
        (
            format!(
                r#"{{"workflow_version_id":"{id}","target":{{"kind":"none"}},"args":{{"dry_run":true,"dry_run":false}}}}"#
            ),
            "duplicate field",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(api("/runs"))
                    .header("content-type", "application/json")
                    .body(Body::from(raw))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response_json!(response, StatusCode::UNPROCESSABLE_ENTITY).await;
        assert_eq!(body["errors"][0]["code"], "run_intent_invalid");
        assert!(
            body["errors"][0]["detail"]
                .as_str()
                .unwrap()
                .contains(expected_detail)
        );
    }
    assert!(state.runs.lock().unwrap().is_empty());
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
async fn post_runs_run_intent_maps_missing_version_environment_and_target_errors() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let missing_version_id = fabro_types::test_support::test_workflow_version_id();
    let base_intent = json!({
        "workflow_version_id": missing_version_id,
        "target": {
            "kind": "git",
            "repo": "fabro-sh/fabro",
            "branch": "feature/run-intent"
        },
        "args": {}
    });

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api("/runs"))
                .header("content-type", "application/json")
                .body(Body::from(base_intent.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::NOT_FOUND).await;
    assert_eq!(body["errors"][0]["code"], "workflow_version_not_found");

    let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;
    for (patch, expected_status, expected_code) in [
        (
            json!({ "environment_id": "missing-environment" }),
            StatusCode::NOT_FOUND,
            "environment_not_found",
        ),
        (
            json!({ "environment_id": "not valid" }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "run_intent_invalid",
        ),
        (
            json!({ "target": { "kind": "git", "repo": "fabro-sh/fabro", "branch": "heads/main" } }),
            StatusCode::UNPROCESSABLE_ENTITY,
            "target_invalid",
        ),
    ] {
        let mut intent = json!({
            "workflow_version_id": workflow_version_id,
            "target": {
                "kind": "git",
                "repo": "fabro-sh/fabro",
                "branch": "feature/run-intent"
            },
            "args": {}
        });
        for (key, value) in patch.as_object().unwrap() {
            intent[key] = value.clone();
        }
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(api("/runs"))
                    .header("content-type", "application/json")
                    .body(Body::from(intent.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = response_json!(response, expected_status).await;
        assert_eq!(body["errors"][0]["code"], expected_code);
    }

    assert!(state.runs.lock().expect("runs lock poisoned").is_empty());
}

#[tokio::test]
async fn post_runs_run_intent_rejects_none_target_with_local_environment_before_persistence() {
    let state = local_test_app_state();
    let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api("/runs"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "workflow_version_id": workflow_version_id,
                        "target": { "kind": "none" },
                        "args": {}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response_json!(response, StatusCode::UNPROCESSABLE_ENTITY).await;
    assert_eq!(body["errors"][0]["code"], "target_environment_unsupported");
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
async fn post_runs_run_intent_rejects_disabled_or_unready_sandbox_integrations() {
    let disabled_state = test_app_state_with_options(
        server_settings_from_toml(
            r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.sandbox.providers.docker]
enabled = false
"#,
        ),
        RunLayer::default(),
        5,
    );
    assert_run_intent_targets_unavailable(&disabled_state).await;

    let daytona_state = TestAppStateBuilder::new()
        .default_environment_provider(Some(SandboxProviderKind::DAYTONA))
        .vault_entries([(fabro_static::EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
        .build();
    assert_run_intent_targets_unavailable(&daytona_state).await;
}

#[tokio::test]
async fn create_run_from_intent_helper_persists_automation_version_and_exact_target() {
    let state = TestAppStateBuilder::new()
        .env_lookup(|_| None)
        .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
        .build();
    let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;
    let run_id = RunId::new();
    let automation = fabro_types::AutomationRef {
        id:              "nightly".to_string(),
        name:            Some("Nightly".to_string()),
        trigger_id:      Some("schedule".to_string()),
        workflow_source: None,
    };
    let target = RunTarget::Git(GitRunTarget {
        repo:   "fabro-sh/fabro".to_string(),
        branch: "main".to_string(),
        tag:    Some("v1.2.3".to_string()),
        sha:    Some("0123456789abcdef0123456789abcdef01234567".to_string()),
    });

    let response = Box::pin(handler::runs::create_run_from_intent(
        Arc::clone(&state),
        handler::runs::CreateRunFromIntentRequest {
            intent:          fabro_api::types::RunIntent {
                workflow_version_id,
                target: target.clone(),
                args: fabro_api::types::RunIntentArgs::default(),
                environment_id: None,
                parent_id: None,
                title: None,
                goal: None,
            },
            explicit_run_id: Some(run_id),
            actor:           Principal::System {
                system_kind: SystemActorKind::Engine,
            },
            headers:         HeaderMap::new(),
            automation:      Some(automation.clone()),
        },
    ))
    .await;

    let body = response_json!(response, StatusCode::CREATED).await;
    assert_eq!(body["automation"]["id"], automation.id);
    let summary = state
        .stores
        .run_summaries
        .get(&run_id, Utc::now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(summary.automation, Some(automation.clone()));
    let run_store = state.stores.runs.open_run_reader(&run_id).await.unwrap();
    let projection = run_store.state().await.unwrap();
    assert_eq!(
        projection.spec.workflow_version_id,
        Some(workflow_version_id)
    );
    assert_eq!(projection.spec.target, Some(target));
    assert_eq!(projection.spec.automation, Some(automation));
    assert_eq!(
        run_store
            .list_events()
            .await
            .unwrap()
            .iter()
            .map(|event| event.event.event_name())
            .collect::<Vec<_>>(),
        ["run.created", "run.submitted"]
    );
}

#[tokio::test]
async fn create_run_from_intent_rejects_git_target_with_dockerfile_only_environment() {
    // Upstream (sandbox-driver adoption): the docker provider speaks
    // `SandboxSource::Image` only, so admission rejects docker environments
    // whose only image source is `image.dockerfile`. The fork's in-engine
    // auto-build (fabro-969f) is retired; run images are provisioned by
    // `just run-images` (deploy time) or a Daytona environment instead.
    let state = TestAppStateBuilder::new()
        .env_lookup(|_| None)
        .vault_entries([(EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
        .build();
    state
        .environment_store()
        .create(fabro_environment::EnvironmentDraft {
            id:       fabro_environment::EnvironmentId::new("toolchain-inline")
                .expect("valid environment id"),
            settings: fabro_types::settings::run::EnvironmentSettings {
                provider: SandboxProviderKind::DOCKER,
                image: fabro_types::settings::run::EnvironmentImageSettings {
                    docker:     None,
                    dockerfile: Some(fabro_types::settings::run::DockerfileSource::Inline(
                        "FROM alpine:3\n".to_string(),
                    )),
                },
                ..fabro_types::settings::run::EnvironmentSettings::default()
            },
        })
        .await
        .expect("inline dockerfile environment should persist");
    let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, None).await;
    let target = RunTarget::Git(GitRunTarget {
        repo:   "fabro-sh/fabro".to_string(),
        branch: "main".to_string(),
        tag:    None,
        sha:    None,
    });

    let response = Box::pin(handler::runs::create_run_from_intent(
        Arc::clone(&state),
        handler::runs::CreateRunFromIntentRequest {
            intent:          fabro_api::types::RunIntent {
                workflow_version_id,
                target: target.clone(),
                args: fabro_api::types::RunIntentArgs::default(),
                environment_id: Some("toolchain-inline".to_string()),
                parent_id: None,
                title: None,
                goal: None,
            },
            explicit_run_id: None,
            actor:           Principal::System {
                system_kind: SystemActorKind::Engine,
            },
            headers:         HeaderMap::new(),
            automation:      None,
        },
    ))
    .await;

    let body = response_json!(response, StatusCode::UNPROCESSABLE_ENTITY).await;
    assert_eq!(body["errors"][0]["code"], "target_environment_unsupported");
}

#[tokio::test]
async fn fake_automation_materializer_injection_captures_input_and_returns_version() {
    let fake = TestAutomationRunMaterializer::succeed(GitRunTarget {
        repo:   "fabro-sh/fabro".to_string(),
        branch: "main".to_string(),
        tag:    None,
        sha:    Some("0123456789abcdef0123456789abcdef01234567".to_string()),
    });
    let state = TestAppStateBuilder::new()
        .automation_materializer(fake.clone())
        .build();
    let run_id = RunId::new();
    let temp_root = PathBuf::from("/tmp/fabro/automation");
    let target = GitRunTarget {
        repo:   "fabro-sh/fabro".to_string(),
        branch: "main".to_string(),
        tag:    None,
        sha:    None,
    };

    let output = state
        .materialize_automation_run(AutomationRunMaterializeInput {
            automation_id: AutomationId::new("nightly").unwrap(),
            target: target.clone(),
            workflow_source: None,
            workflow: "demo".to_string(),
            run_id,
            temp_root: temp_root.clone(),
        })
        .await
        .expect("fake materializer should succeed");

    let stored = fabro_workflow_version::WorkflowVersionStore::new(state.store_ref().blobs())
        .get(&output.workflow_version_id)
        .await
        .unwrap();
    assert!(stored.is_some());
    let captured = fake.captured_inputs();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].automation_id.as_str(), "nightly");
    assert_eq!(captured[0].target, target);
    assert_eq!(captured[0].workflow, "demo");
    assert_eq!(captured[0].run_id, run_id);
    assert_eq!(captured[0].temp_root, temp_root);
}

#[tokio::test]
async fn list_run_stages_projects_retrying_until_completion() {
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
        "setup",
        1,
        &workflow_event::Event::StageStarted {
            graph_visit:           None,
            resumed_from_stage_id: None,
            node_id:               "setup".to_string(),
            name:                  "Setup".to_string(),
            index:                 0,
            handler_type:          "command".to_string(),
            attempt:               1,
            max_attempts:          1,
        },
    )
    .await;
    append_scoped_stage_event(
        &state,
        run_id,
        "setup",
        1,
        &workflow_event::Event::StageCompleted {
            node_id: "setup".to_string(),
            name: "Setup".to_string(),
            index: 0,
            timing: fabro_types::StageTiming::wall_only(5),
            status: "succeeded".to_string(),
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
    )
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
            index:          1,
            failure:        FailureDetail::new("try again", FailureCategory::TransientInfra),
            will_retry:     true,
            timing:         fabro_types::StageTiming::wall_only(10),
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
            index:        1,
            attempt:      2,
            max_attempts: 3,
            delay_ms:     100,
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
    assert_eq!(stage_status(&body, "setup@1"), "succeeded");
    assert_eq!(stage_status(&body, "work@1"), "retrying");

    append_scoped_stage_event(
        &state,
        run_id,
        "work",
        1,
        &workflow_event::Event::StageCompleted {
            node_id: "work".to_string(),
            name: "Work".to_string(),
            index: 1,
            timing: fabro_types::StageTiming::wall_only(25),
            status: "partially_succeeded".to_string(),
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
            attempt: 2,
            max_attempts: 3,
        },
    )
    .await;

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
    assert_eq!(stage_status(&body, "work@1"), "partially_succeeded");
}

#[tokio::test]
async fn list_run_stages_distinguishes_visits() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    let mut graph = Graph::new("test");
    let mut verify = Node::new("verify");
    verify
        .attrs
        .insert("type".to_string(), AttrValue::String("command".to_string()));
    graph.nodes.insert("verify".to_string(), verify);

    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunCreated {
            run_id,
            title: None,
            settings: serde_json::to_value(fabro_types::WorkflowSettings::default()).unwrap(),
            graph: serde_json::to_value(&graph).unwrap(),
            workflow_source: None,
            labels: std::collections::BTreeMap::default(),
            source_directory: None,
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
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ])
    .await;

    // First visit of `verify` — failed.
    append_scoped_stage_event(
        &state,
        run_id,
        "verify",
        1,
        &workflow_event::Event::StageStarted {
            graph_visit:           None,
            resumed_from_stage_id: None,
            node_id:               "verify".to_string(),
            name:                  "Verify".to_string(),
            index:                 1,
            handler_type:          "command".to_string(),
            attempt:               1,
            max_attempts:          1,
        },
    )
    .await;
    append_scoped_stage_event(
        &state,
        run_id,
        "verify",
        1,
        &workflow_event::Event::StageCompleted {
            node_id: "verify".to_string(),
            name: "Verify".to_string(),
            index: 1,
            timing: fabro_types::StageTiming::wall_only(1500),
            status: "failed".to_string(),
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
    )
    .await;

    // Second visit of `verify` — running.
    append_scoped_stage_event(
        &state,
        run_id,
        "verify",
        2,
        &workflow_event::Event::StageStarted {
            graph_visit:           None,
            resumed_from_stage_id: None,
            node_id:               "verify".to_string(),
            name:                  "Verify".to_string(),
            index:                 1,
            handler_type:          "command".to_string(),
            attempt:               1,
            max_attempts:          1,
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

    let data = body["data"].as_array().unwrap();
    let verify_entries: Vec<_> = data.iter().filter(|s| s["node_id"] == "verify").collect();
    assert_eq!(verify_entries.len(), 2, "expected two verify visits");

    let first = stage_entry(&body, "verify@1");
    assert_eq!(first["node_id"], "verify");
    assert_eq!(first["visit"], 1);
    assert_eq!(first["handler"], "command");
    assert_eq!(first["status"], "failed");
    assert_eq!(first["wall_time_ms"], 1500);

    let second = stage_entry(&body, "verify@2");
    assert_eq!(second["node_id"], "verify");
    assert_eq!(second["visit"], 2);
    assert_eq!(second["handler"], "command");
    assert_eq!(second["status"], "running");

    // Old `dot_id` field must be gone.
    assert!(first.get("dot_id").is_none(), "dot_id should be removed");
}

#[tokio::test]
async fn list_run_stages_exposes_execution_identity_for_resumed_stage() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    let mut graph = Graph::new("test");
    let mut work = Node::new("work");
    work.attrs
        .insert("type".to_string(), AttrValue::String("agent".to_string()));
    graph.nodes.insert("work".to_string(), work);

    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunCreated {
            run_id,
            title: None,
            settings: serde_json::to_value(fabro_types::WorkflowSettings::default()).unwrap(),
            graph: serde_json::to_value(&graph).unwrap(),
            workflow_source: None,
            labels: std::collections::BTreeMap::default(),
            source_directory: None,
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
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ])
    .await;

    // Legacy-shaped first execution without identity metadata.
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
    // Reexecution after cancel/resume: same graph visit, next ordinal.
    append_scoped_stage_event(
        &state,
        run_id,
        "work",
        2,
        &workflow_event::Event::StageStarted {
            graph_visit:           Some(1),
            resumed_from_stage_id: Some(fabro_types::StageId::new("work", 1)),
            node_id:               "work".to_string(),
            name:                  "Work".to_string(),
            index:                 1,
            handler_type:          "agent".to_string(),
            attempt:               1,
            max_attempts:          1,
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

    let first = stage_entry(&body, "work@1");
    assert!(
        first.get("graph_visit").is_none(),
        "legacy stage should omit graph_visit"
    );
    assert!(
        first.get("resumed_from_stage_id").is_none(),
        "legacy stage should omit resumed_from_stage_id"
    );

    let second = stage_entry(&body, "work@2");
    assert_eq!(second["visit"], 2);
    assert_eq!(second["graph_visit"], 1);
    assert_eq!(second["resumed_from_stage_id"], "work@1");
}

#[tokio::test]
async fn list_run_stages_exposes_parallel_branch_identity() {
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
        "ordinary",
        1,
        &workflow_event::Event::StageStarted {
            graph_visit:           Some(1),
            resumed_from_stage_id: None,
            node_id:               "ordinary".to_string(),
            name:                  "Ordinary".to_string(),
            index:                 0,
            handler_type:          "agent".to_string(),
            attempt:               1,
            max_attempts:          1,
        },
    )
    .await;

    let parallel_group_id = StageId::new("review_fork", 2);
    let parallel_branch_id = ParallelBranchId::new(parallel_group_id.clone(), 4);
    let branch_event = workflow_event::Event::ParallelBranchStarted {
        parallel_group_id:     parallel_group_id.clone(),
        parallel_branch_id:    parallel_branch_id.clone(),
        branch:                "review_glm".to_string(),
        index:                 4,
        item_label:            None,
        graph_visit:           Some(3),
        resumed_from_stage_id: None,
    };
    let branch_scope = workflow_event::StageScope::for_parallel_branch(
        "review_glm",
        3,
        parallel_group_id,
        parallel_branch_id,
    );
    append_event_with_scope(&state, run_id, &branch_event, &branch_scope).await;

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

    let branch = stage_entry(&body, "review_glm@3");
    assert_eq!(branch["parallel_group_id"], "review_fork@2");
    assert_eq!(branch["parallel_branch_index"], 4);

    let ordinary = stage_entry(&body, "ordinary@1");
    assert!(ordinary.get("parallel_group_id").is_none());
    assert!(ordinary.get("parallel_branch_index").is_none());
}

/// The stage popover reads `usage` off the stages list, so it must be scoped
/// to one visit — unlike the Usage tab's rows, which sum every visit of a
/// node.
#[tokio::test]
async fn list_run_stages_reports_usage_per_visit() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();

    create_priced_retry_run(&state, run_id).await;

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

    let first = stage_entry(&body, "verify@1");
    assert_eq!(first["usage"]["tokens"]["input"], 100);
    assert_eq!(first["usage"]["tokens"]["output"], 10);
    assert_eq!(first["usage"]["cost"]["usd_micros"], 110);

    let second = stage_entry(&body, "verify@2");
    assert_eq!(second["usage"]["tokens"]["input"], 200);
    assert_eq!(second["usage"]["tokens"]["output"], 20);
    assert_eq!(second["usage"]["cost"]["usd_micros"], 220);
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
async fn get_run_status_returns_status() {
    let state = test_app_state();
    let app = test_app_with_scheduler(state);

    let run_id = create_and_start_run(&app, MINIMAL_DOT).await;

    // Give run a moment to start
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Check status
    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(run_json_id(&body).unwrap(), run_id);
    assert_eq!(body["goal"].as_str().unwrap(), "Test");
    assert_eq!(body["title"].as_str().unwrap(), "Test");
    assert!(body["repository"].is_object());
    assert!(!body["repository"]["name"].as_str().unwrap().is_empty());
    assert!(body["timestamps"]["created_at"].is_string());
    assert!(body["labels"].is_object());
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
async fn get_run_status_not_found() {
    let app = test_app_with();
    let missing_run_id = fixtures::RUN_64;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{missing_run_id}")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn resolve_run_returns_unique_run_id_prefix_match() {
    let app = test_app_with();
    let run_id = create_run(&app, MINIMAL_DOT).await;
    let selector = &run_id[..8];

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/resolve?selector={selector}")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(run_json_id(&body), Some(run_id.as_str()));
}

#[tokio::test]
async fn resolve_run_returns_bad_request_for_ambiguous_prefix() {
    let app = test_app_with();
    let run_id_a = create_run(&app, MINIMAL_DOT).await;
    let run_id_b = create_run(&app, MINIMAL_DOT).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api("/runs/resolve?selector=0"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response_json!(response, StatusCode::BAD_REQUEST).await;
    let detail = body["errors"][0]["detail"]
        .as_str()
        .expect("error detail should be present");
    assert!(
        detail.contains(&run_id_a),
        "detail should mention first run: {detail}"
    );
    assert!(
        detail.contains(&run_id_b),
        "detail should mention second run: {detail}"
    );
    assert!(
        detail.contains("created_at="),
        "detail should include creation timestamps: {detail}"
    );
    assert!(
        detail.contains("workflow="),
        "detail should include workflow names: {detail}"
    );
    assert!(
        detail.contains("origin="),
        "detail should include origin URLs: {detail}"
    );
}

#[tokio::test]
async fn resolve_run_prefers_most_recent_exact_workflow_slug_match() {
    let app = test_app_with();
    let older_id = create_run_for_target(
        &app,
        "ship-feature.fabro",
        &named_workflow_dot("ShipFeatureAlpha", "older"),
    )
    .await;
    let newer_id = create_run_for_target(
        &app,
        "ship-feature.fabro",
        &named_workflow_dot("ShipFeatureBeta", "newer"),
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api("/runs/resolve?selector=ship-feature"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(run_json_id(&body), Some(newer_id.as_str()));
    assert_ne!(run_json_id(&body), Some(older_id.as_str()));
}

#[tokio::test]
async fn resolve_run_prefers_most_recent_collapsed_workflow_name_match() {
    let app = test_app_with();
    let older_id = create_run_for_target_with_workflow_name(
        &app,
        "nightly-alpha.fabro",
        &named_workflow_dot("OlderNightlyGraph", "older"),
        "Nightly_Build",
    )
    .await;
    let newer_id = create_run_for_target_with_workflow_name(
        &app,
        "nightly-beta.fabro",
        &named_workflow_dot("NewerNightlyGraph", "newer"),
        "Nightly_Build",
    )
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api("/runs/resolve?selector=nightlybuild"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(run_json_id(&body), Some(newer_id.as_str()));
    assert_ne!(run_json_id(&body), Some(older_id.as_str()));
}

#[tokio::test]
async fn resolve_run_returns_not_found_for_unknown_selector() {
    let app = test_app_with();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api("/runs/resolve?selector=missing-run"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn get_run_state_returns_projection() {
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
        .uri(api(&format!("/runs/{run_id}/state")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert!(body["stages"].is_object());
}

#[tokio::test]
async fn get_run_logs_returns_per_run_log_file() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_durable_run_with_events(&state, run_id, &[workflow_event::Event::RunSubmitted {
        definition_blob: None,
    }])
    .await;
    let log_path = Storage::new(state.server_storage_dir())
        .run_scratch(&run_id)
        .runtime_dir()
        .join("server.log");
    tokio::fs::create_dir_all(log_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&log_path, b"worker log line\nsecond line\n")
        .await
        .unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/logs")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = response_bytes!(response, StatusCode::OK).await;

    assert_eq!(content_type.as_deref(), Some("text/plain; charset=utf-8"));
    assert_eq!(&body[..], b"worker log line\nsecond line\n");
}

#[tokio::test]
async fn get_run_logs_returns_not_found_for_missing_run() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(state);
    let missing_run_id = RunId::new();

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{missing_run_id}/logs")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn get_run_logs_returns_not_found_when_log_file_is_missing() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_durable_run_with_events(&state, run_id, &[workflow_event::Event::RunSubmitted {
        definition_blob: None,
    }])
    .await;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/logs")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn get_run_stage_command_log_returns_scratch_slice() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    let stage_id = StageId::new("script_node", 1);
    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::StageStarted {
            graph_visit:           None,
            resumed_from_stage_id: None,
            node_id:               "script_node".to_string(),
            name:                  "Script".to_string(),
            index:                 1,
            handler_type:          "command".to_string(),
            attempt:               1,
            max_attempts:          1,
        },
        workflow_event::Event::CommandStarted {
            node_id:    "script_node".to_string(),
            script:     "echo hello world".to_string(),
            command:    "echo hello world".to_string(),
            language:   "shell".to_string(),
            timeout_ms: None,
        },
    ])
    .await;
    let run_dir = Storage::new(state.server_storage_dir())
        .run_scratch(&run_id)
        .root()
        .to_path_buf();
    let log_path = command_log_path(&run_dir, &stage_id);
    tokio::fs::create_dir_all(log_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&log_path, b"hello world").await.unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/stages/{stage_id}/logs/output?offset=6&limit=5"
        )))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let bytes = BASE64_STANDARD
        .decode(body["bytes_base64"].as_str().unwrap())
        .unwrap();

    assert!(body.get("stream").is_none());
    assert_eq!(body["offset"], 6);
    assert_eq!(body["next_offset"], 11);
    assert_eq!(body["total_bytes"], 11);
    assert_eq!(bytes, b"world");
    assert_eq!(body["eof"], false);
    assert_eq!(body["cas_ref"], serde_json::Value::Null);
    assert_eq!(body["live_streaming"], true);
}

#[tokio::test]
async fn get_run_stage_command_log_returns_cas_slice() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    let run_store = state.stores.runs.create_run(&run_id).await.unwrap();
    append_default_run_created(&run_store, run_id).await;
    let output_blob = run_store
        .write_blob(&serde_json::to_vec("hello world").unwrap())
        .await
        .unwrap();
    let output_ref = format!("blob://sha256/{output_blob}");
    for event in [
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::StageStarted {
            graph_visit:           None,
            resumed_from_stage_id: None,
            node_id:               "script_node".to_string(),
            name:                  "Script".to_string(),
            index:                 1,
            handler_type:          "command".to_string(),
            attempt:               1,
            max_attempts:          1,
        },
        workflow_event::Event::CommandCompleted {
            node_id:        "script_node".to_string(),
            output:         output_ref.clone(),
            exit_code:      Some(0),
            duration_ms:    5,
            termination:    CommandTermination::Exited,
            output_bytes:   11,
            live_streaming: false,
        },
    ] {
        workflow_event::append_event(&run_store, &run_id, &event)
            .await
            .unwrap();
    }

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/stages/script_node@1/logs/output?offset=6&limit=5"
        )))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let bytes = BASE64_STANDARD
        .decode(body["bytes_base64"].as_str().unwrap())
        .unwrap();

    assert!(body.get("stream").is_none());
    assert_eq!(body["offset"], 6);
    assert_eq!(body["next_offset"], 11);
    assert_eq!(body["total_bytes"], 11);
    assert_eq!(bytes, b"world");
    assert_eq!(body["eof"], true);
    assert_eq!(body["cas_ref"], output_ref);
    assert_eq!(body["live_streaming"], false);
}

#[tokio::test]
async fn get_run_stage_command_log_prefers_scratch_when_cas_ref_exists() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    let stage_id = StageId::new("script_node", 1);
    let run_store = state.stores.runs.create_run(&run_id).await.unwrap();
    append_default_run_created(&run_store, run_id).await;
    let output_blob = run_store
        .write_blob(&serde_json::to_vec("cas log").unwrap())
        .await
        .unwrap();
    let output_ref = format!("blob://sha256/{output_blob}");
    for event in [
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::StageStarted {
            graph_visit:           None,
            resumed_from_stage_id: None,
            node_id:               "script_node".to_string(),
            name:                  "Script".to_string(),
            index:                 1,
            handler_type:          "command".to_string(),
            attempt:               1,
            max_attempts:          1,
        },
        workflow_event::Event::CommandCompleted {
            node_id:        "script_node".to_string(),
            output:         output_ref.clone(),
            exit_code:      Some(0),
            duration_ms:    5,
            termination:    CommandTermination::Exited,
            output_bytes:   7,
            live_streaming: false,
        },
    ] {
        workflow_event::append_event(&run_store, &run_id, &event)
            .await
            .unwrap();
    }

    let run_dir = Storage::new(state.server_storage_dir())
        .run_scratch(&run_id)
        .root()
        .to_path_buf();
    let log_path = command_log_path(&run_dir, &stage_id);
    tokio::fs::create_dir_all(log_path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&log_path, b"scratch log").await.unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/stages/{stage_id}/logs/output?offset=0&limit=64"
        )))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let bytes = BASE64_STANDARD
        .decode(body["bytes_base64"].as_str().unwrap())
        .unwrap();

    assert!(body.get("stream").is_none());
    assert_eq!(body["offset"], 0);
    assert_eq!(body["next_offset"], 11);
    assert_eq!(body["total_bytes"], 11);
    assert_eq!(bytes, b"scratch log");
    assert_eq!(body["eof"], true);
    assert_eq!(body["cas_ref"], output_ref);
    assert_eq!(body["live_streaming"], false);
}

#[tokio::test]
async fn get_run_stage_command_log_returns_not_found_for_missing_stage() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_durable_run_with_events(&state, run_id, &[workflow_event::Event::RunSubmitted {
        definition_blob: None,
    }])
    .await;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/stages/missing@1/logs/output")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn get_run_stage_context_window_returns_not_found_for_missing_run() {
    let app = crate::test_support::build_test_router(test_app_state_with_isolated_storage());
    let run_id = RunId::new();

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!(
                    "/runs/{run_id}/stages/agent@1/context-window"
                )))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn get_run_stage_context_window_returns_not_found_for_missing_stage() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_durable_run_with_events(&state, run_id, &[workflow_event::Event::RunSubmitted {
        definition_blob: None,
    }])
    .await;

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!(
                    "/runs/{run_id}/stages/missing@1/context-window"
                )))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn get_run_stage_context_window_returns_unavailable_for_non_agent_stage() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        stage_started_event("script_node", "command"),
        command_started_event("script_node"),
    ])
    .await;

    let body = response_json!(
        app.oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!(
                    "/runs/{run_id}/stages/script_node@1/context-window"
                )))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap(),
        StatusCode::OK
    )
    .await;

    assert_eq!(body["available"], false);
    assert_eq!(body["unavailable_reason"], "not_agent_stage");
    assert_eq!(body["breakdown"], json!([]));
    assert_eq!(body["staleness"], "unavailable");
}

#[tokio::test]
async fn get_run_stage_context_window_returns_not_observed_for_agent_stage_without_snapshot() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        agent_session_activated_event("agent_node", 1),
    ])
    .await;

    let body = response_json!(
        app.oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!(
                    "/runs/{run_id}/stages/agent_node@1/context-window"
                )))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap(),
        StatusCode::OK
    )
    .await;

    assert_eq!(body["available"], false);
    assert_eq!(body["unavailable_reason"], "not_observed");
    assert_eq!(body["input_tokens"], serde_json::Value::Null);
    assert!(!body["warnings"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn get_run_stage_context_window_returns_live_projected_snapshot() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        stage_started_event("agent_node", "agent"),
        context_window_event(
            "agent_node",
            1,
            context_window_snapshot(123_456, Vec::new()),
        ),
    ])
    .await;

    let body = response_json!(
        app.oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!(
                    "/runs/{run_id}/stages/agent_node@1/context-window"
                )))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap(),
        StatusCode::OK
    )
    .await;

    assert_eq!(body["stage_id"], "agent_node@1");
    assert_eq!(body["available"], true);
    assert_eq!(body["provider"], "openai");
    assert_eq!(body["count_method"], "response_usage_scaled_breakdown");
    assert_eq!(body["staleness"], "live");
    assert_eq!(body["input_tokens"], 123_456);
    assert_eq!(body["breakdown"][0]["category"], "conversation");
}

#[tokio::test]
async fn get_run_stage_context_window_marks_completed_stage_snapshot_stored() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        stage_started_event("agent_node", "agent"),
        context_window_event("agent_node", 1, context_window_snapshot(100, Vec::new())),
        stage_completed_event("agent_node"),
    ])
    .await;

    let body = response_json!(
        app.oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!(
                    "/runs/{run_id}/stages/agent_node@1/context-window"
                )))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap(),
        StatusCode::OK
    )
    .await;

    assert_eq!(body["available"], true);
    assert_eq!(body["staleness"], "stored");
    assert_eq!(body["input_tokens"], 100);
}

#[tokio::test]
async fn get_run_stage_context_window_returns_projected_warnings() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();
    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        stage_started_event("agent_node", "agent"),
        context_window_event(
            "agent_node",
            1,
            context_window_snapshot(100, vec![ContextWindowWarning {
                code:    "provider_token_count_failed".to_string(),
                message: "provider input token counting failed; returned local estimate"
                    .to_string(),
            }]),
        ),
    ])
    .await;

    let body = response_json!(
        app.oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!(
                    "/runs/{run_id}/stages/agent_node@1/context-window"
                )))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap(),
        StatusCode::OK
    )
    .await;

    assert_eq!(body["warnings"][0]["code"], "provider_token_count_failed");
}

#[tokio::test]
async fn restarted_run_state_details_load_from_sql_and_preserve_error_statuses() {
    let object_store: Arc<dyn object_store::ObjectStore> =
        Arc::new(object_store::memory::InMemory::new());
    let summaries = fabro_store::test_support::test_run_summary_store();
    let blobs = fabro_store::test_support::test_blob_store();
    let first_state = test_app_state_over_shared_stores(&object_store, &blobs, &summaries);
    let healthy_id = fixtures::RUN_1;
    let broken_id = fixtures::RUN_2;
    create_durable_run_with_events(&first_state, healthy_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
    ])
    .await;
    create_succeeded_run(&first_state, broken_id).await;
    first_state
        .stores
        .run_summaries
        .test_delete_run_events(&broken_id)
        .await
        .unwrap();
    drop(first_state);

    let reopened_state = test_app_state_over_shared_stores(&object_store, &blobs, &summaries);
    assert_eq!(
        reconcile_incomplete_runs_on_startup(&reopened_state)
            .await
            .unwrap(),
        0,
        "startup reconciliation must not replay terminal histories"
    );
    let app = crate::test_support::build_test_router(reopened_state);

    let healthy = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{healthy_id}/state")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let healthy_body = response_json!(healthy, StatusCode::OK).await;
    assert_eq!(healthy_body["spec"]["run_id"], healthy_id.to_string());

    let missing = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{}/state", fixtures::RUN_3)))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(missing, StatusCode::NOT_FOUND).await;

    let broken = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{broken_id}/state")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(broken, StatusCode::INTERNAL_SERVER_ERROR).await;
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
async fn get_run_state_includes_provenance_from_user_agent() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let req = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .header("user-agent", "fabro-cli/1.2.3")
        .body(intent_body(&app, MINIMAL_DOT).await)
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    let run_id = body["id"].as_str().unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/state")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(
        body["spec"]["provenance"]["server"]["version"],
        FABRO_VERSION
    );
    assert_eq!(
        body["spec"]["provenance"]["client"]["user_agent"],
        "fabro-cli/1.2.3"
    );
    assert_eq!(body["spec"]["provenance"]["client"]["name"], "fabro-cli");
    assert_eq!(body["spec"]["provenance"]["client"]["version"], "1.2.3");
    assert_eq!(body["spec"]["provenance"]["subject"]["kind"], "user");
    assert_eq!(
        body["spec"]["provenance"]["subject"]["auth_method"],
        "dev_token"
    );
    assert_eq!(body["spec"]["provenance"]["subject"]["login"], "dev");
    assert_eq!(
        body["spec"]["provenance"]["subject"]["identity"]["issuer"],
        "fabro:dev"
    );
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

#[tokio::test]
async fn get_checkpoint_returns_null_initially() {
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

    // Get checkpoint immediately (before run completes, may be null)
    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/checkpoint")))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    checked_response!(response, StatusCode::OK).await;
}

#[tokio::test]
async fn write_and_read_run_blob_accepts_uppercase_hash() {
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
        .uri(api(&format!("/runs/{run_id}/blobs")))
        .header("content-type", "application/octet-stream")
        .body(Body::from("hello blob"))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let blob_hash = body["hash"].as_str().unwrap();

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/blobs/{}",
            blob_hash.to_uppercase()
        )))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let bytes = response_bytes!(response, StatusCode::OK).await;
    assert_eq!(&bytes[..], b"hello blob");
}

#[tokio::test]
async fn stage_artifacts_round_trip() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id = create_run(&app, MINIMAL_DOT).await;
    let stage_id = "code@2";

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!(
            "/runs/{run_id}/stages/{stage_id}/artifacts?filename=src/lib.rs&retry=1"
        )))
        .header("content-type", "application/octet-stream")
        .body(Body::from("fn main() {}"))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NO_CONTENT).await;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/stages/{stage_id}/artifacts")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["data"][0]["filename"], "src/lib.rs");
    assert_eq!(body["data"][0]["retry"], 1);
    assert_eq!(body["data"][0]["size"], 12);

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/stages/{stage_id}/artifacts/download?filename=src/lib.rs"
        )))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::BAD_REQUEST).await;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/stages/{stage_id}/artifacts/download?filename=src/lib.rs&retry=1"
        )))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let bytes = response_bytes!(response, StatusCode::OK).await;
    assert_eq!(&bytes[..], b"fn main() {}");
}

#[tokio::test]
async fn stage_artifacts_keep_same_filename_per_retry() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id = create_run(&app, MINIMAL_DOT).await;
    let stage_id = "code@2";

    for (retry, body) in [(1, "first"), (2, "second")] {
        let req = Request::builder()
            .method("POST")
            .uri(api(&format!(
                "/runs/{run_id}/stages/{stage_id}/artifacts?filename=logs/output.txt&retry={retry}"
            )))
            .header("content-type", "application/octet-stream")
            .body(Body::from(body))
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        assert_status!(response, StatusCode::NO_CONTENT).await;
    }

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/stages/{stage_id}/artifacts")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["data"][0]["filename"], "logs/output.txt");
    assert_eq!(body["data"][0]["retry"], 1);
    assert_eq!(body["data"][1]["filename"], "logs/output.txt");
    assert_eq!(body["data"][1]["retry"], 2);

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/stages/{stage_id}/artifacts/download?filename=logs/output.txt&retry=2"
        )))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let bytes = response_bytes!(response, StatusCode::OK).await;
    assert_eq!(&bytes[..], b"second");
}

#[tokio::test]
async fn run_artifacts_download_streams_latest_files_as_zip() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();

    let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
    for event in [
        workflow_event::Event::RunRunnable {
            source: fabro_types::RunRunnableSource::StartRequested,
            actor:  None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ] {
        workflow_event::append_event(&run_store, &run_id, &event)
            .await
            .unwrap();
    }
    append_scoped_stage_event(
        &state,
        run_id,
        "build",
        1,
        &stage_started_event("build", "command"),
    )
    .await;
    append_scoped_stage_event(
        &state,
        run_id,
        "verify",
        1,
        &stage_started_event("verify", "command"),
    )
    .await;

    for (stage_id, retry, path, contents) in [
        (
            StageId::new("unknown", 1),
            99,
            "reports/result.txt",
            &b"unknown stage"[..],
        ),
        (
            StageId::new("build", 1),
            1,
            "reports/result.txt",
            &b"build result"[..],
        ),
        (
            StageId::new("verify", 1),
            1,
            "reports/result.txt",
            &b"first verify"[..],
        ),
        (
            StageId::new("verify", 1),
            2,
            "reports/result.txt",
            &b"latest verify"[..],
        ),
        (
            StageId::new("build", 1),
            1,
            "logs/run.txt",
            &b"build log"[..],
        ),
        (
            StageId::new("start", 1),
            1,
            "control-start.txt",
            &b"excluded"[..],
        ),
        (
            StageId::new("exit", 1),
            1,
            "control-exit.txt",
            &b"excluded"[..],
        ),
        // Neither stage reached the projection, so both rank equally on stage
        // order and retry. The serialized stage ID breaks the tie the same way
        // the artifacts page does: "unknown@2" sorts above "unknown@10".
        (
            StageId::new("unknown", 10),
            1,
            "orphan.txt",
            &b"visit ten"[..],
        ),
        (
            StageId::new("unknown", 2),
            1,
            "orphan.txt",
            &b"visit two"[..],
        ),
    ] {
        state
            .artifact_store
            .put(&run_id, &ArtifactKey::new(stage_id, retry, path), contents)
            .await
            .unwrap();
    }

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}/artifacts/download")))
                .header(header::ACCEPT_ENCODING, "gzip")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("application/zip")
    );
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok()),
        Some(format!("attachment; filename=\"fabro-artifacts-{run_id}.zip\"").as_str())
    );
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|value| value.to_str().ok()),
        Some("private, no-store")
    );
    assert!(response.headers().get(header::CONTENT_ENCODING).is_none());

    let bytes = response_bytes!(response, StatusCode::OK).await;
    let archive = ZipFileReader::new(bytes).await.unwrap();
    let names = archive
        .file()
        .entries()
        .iter()
        .map(|entry| entry.filename().as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(names, vec![
        "logs/run.txt",
        "orphan.txt",
        "reports/result.txt"
    ]);

    let mut contents_by_name = HashMap::new();
    for (index, name) in names.into_iter().enumerate() {
        let mut entry = archive.reader_with_entry(index).await.unwrap();
        let mut contents = Vec::new();
        entry.read_to_end_checked(&mut contents).await.unwrap();
        contents_by_name.insert(name, contents);
    }
    assert_eq!(contents_by_name["logs/run.txt"], b"build log");
    assert_eq!(contents_by_name["reports/result.txt"], b"latest verify");
    assert_eq!(contents_by_name["orphan.txt"], b"visit two");
}

#[tokio::test]
async fn run_artifacts_download_returns_not_found_for_unknown_run() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{}/artifacts/download", RunId::new())))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn create_run_persists_run_spec() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id = create_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();
    let run_state = state
        .stores
        .runs
        .open_run_reader(&run_id)
        .await
        .unwrap()
        .state()
        .await
        .unwrap();

    assert_eq!(run_state.spec.graph.name, "Test");
}

#[tokio::test]
async fn create_run_keeps_missing_project_and_workflow_names_absent() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let intent = test_intent_with_bearer(
        &app,
        "workflow.fabro",
        "digraph Demo { start [shape=Mdiamond] exit [shape=Msquare] start -> exit }",
        Some("_version = 1\n"),
        None,
    )
    .await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(api("/runs"))
                .header("content-type", "application/json")
                .body(Body::from(intent.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = body_json(response.into_body()).await;
    let run_id = body["id"].as_str().unwrap().parse::<RunId>().unwrap();

    let run_state = state
        .stores
        .runs
        .open_run_reader(&run_id)
        .await
        .unwrap()
        .state()
        .await
        .unwrap();

    assert_eq!(run_state.spec.settings.project.name.as_deref(), None);
    assert_eq!(run_state.spec.settings.workflow.name.as_deref(), None);
    assert_eq!(run_state.spec.graph_name(), Some("Demo"));
}

#[tokio::test]
async fn stage_artifact_upload_rejects_invalid_filename() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id = create_run(&app, MINIMAL_DOT).await;

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!(
            "/runs/{run_id}/stages/code@2/artifacts?filename=../escape.txt&retry=1"
        )))
        .header("content-type", "application/octet-stream")
        .body(Body::from("nope"))
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn fabro_ask_proves_the_wire_path_and_analyst_sentinels() {
    use fabro_client::{Client as ServerClient, Credential as ClientCredential, ServerTarget};
    use fabro_tool::{FabroAskParams, ValidatedAsk, ask_run};

    let sandbox_dir = tempfile::tempdir().unwrap();
    let sandbox_path = sandbox_dir.path().to_path_buf();

    // Scripted LLM: the strict mock carries the ANALYST SESSION sentinels
    // as matchers — positive (resolved model id + the five policy-allowed
    // tools) and negative (no write/shell tools reach the provider).
    let mock_llm = httpmock::MockServer::start();
    let state = crate::test_support::jwt_auth_state_with_openai_base_url(
        &mock_llm.base_url(),
        TEST_SESSION_SECRET,
    );
    // Resolve the model the way the server does for a session without an
    // explicit pick: the default among providers with credentials (only
    // openai has a vault key here).
    let eligible = state
        .resolve_llm_client()
        .await
        .expect("test LLM client should resolve")
        .provider_ids()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    let expected_model =
        fabro_llm::selection::resolve_selection(state.catalog().as_ref(), None, None, &eligible)
            .expect("a default model should resolve")
            .model;
    let llm_post = mock_llm.mock(|when, then| {
        when.method(httpmock::Method::POST)
            .body_includes(format!(r#""model":"{expected_model}""#))
            .body_includes("fabro_run_events")
            .body_includes("fabro_run_get")
            // The native read tools serialize under the profile's own
            // vocabulary ("read…" family), not the canonical names.
            .body_includes("read")
            // Negative sentinels: quoted names must not appear anywhere —
            // the ask-fabro policy denies every write/execute tool.
            .body_excludes(r#""write_file""#)
            .body_excludes(r#""shell""#)
            .body_excludes(r#""spawn_agent""#);
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(
                [
                    r#"data: {"type":"response.output_text.delta","delta":"The gate timed out waiting for approval."}"#,
                    "\n\n",
                    r#"data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","output":[],"usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}}}"#,
                    "\n\n",
                ]
                .concat(),
            );
    });

    // Spawn the real router on a socket so the worker-credentialed
    // fabro_client can reach it; the daemon record points the analyst's
    // own API client at the same server.
    let app = build_router(Arc::clone(&state), jwt_auth_mode());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("test server should bind");
    let addr = listener
        .local_addr()
        .expect("test server should have a local address");
    let runtime_directory =
        fabro_config::Storage::new(state.server_storage_dir()).runtime_directory();
    ServerDaemon::new(
        std::process::id(),
        Bind::Tcp(addr),
        runtime_directory.log_path(),
    )
    .write(&runtime_directory)
    .unwrap();
    let server_task = tokio::spawn(async move {
        let result = axum::serve(listener, app.into_make_service()).await;
        if let Err(err) = result {
            tracing::debug!(error = %err, "test ask server stopped");
        }
    });

    // Terminal target runs: develop (in scope) and nightly (out of scope).
    let terminal_run_with_slug = |slug: &'static str| {
        let state = Arc::clone(&state);
        let sandbox_path = sandbox_path.clone();
        async move {
            let run_id = RunId::new();
            let run_store = state.stores.runs.create_run(&run_id).await.unwrap();
            for event in [
                workflow_event::Event::RunCreated {
                    run_id,
                    title: None,
                    settings: serde_json::to_value(fabro_types::WorkflowSettings::default())
                        .unwrap(),
                    graph: serde_json::to_value(Graph::new(slug)).unwrap(),
                    workflow_source: None,
                    labels: std::collections::BTreeMap::default(),
                    source_directory: None,
                    workflow_slug: Some(slug.to_string()),
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
                workflow_event::Event::SandboxInitialized {
                    provider:          SandboxProviderKind::LOCAL,
                    // The Host provider attaches only to real directories
                    // whose canonical path matches the id it derives from
                    // them, so the record uses a real temp dir.
                    id:                fabro_sandbox::test_support::local_sandbox_id(&sandbox_path)
                        .await,
                    working_directory: sandbox_path.to_string_lossy().to_string(),
                    image:             None,
                    snapshot:          None,
                    repo_cloned:       None,
                    clone_origin_url:  None,
                    clone_branch:      None,
                    workspace_root:    None,
                    repos_root:        None,
                    primary_repo_path: None,
                    primary_repo_link: None,
                },
                workflow_event::Event::RunPending {
                    reason: fabro_types::PendingReason::ApprovalRequired,
                    actor:  None,
                },
                workflow_event::Event::RunRunnable {
                    source: fabro_types::RunRunnableSource::StartRequested,
                    actor:  None,
                },
                workflow_event::Event::RunStarting,
                workflow_event::Event::RunRunning,
                workflow_event::Event::WorkflowRunCompleted {
                    timing:               fabro_types::RunTiming::default(),
                    artifact_count:       0,
                    status:               "succeeded".to_string(),
                    reason:               fabro_types::SuccessReason::Completed,
                    failure:              None,
                    final_git_commit_sha: None,
                    final_patch:          None,
                    diff_summary:         None,
                    usage:                None,
                },
            ] {
                workflow_event::append_event(&run_store, &run_id, &event)
                    .await
                    .unwrap();
            }
            run_id
        }
    };
    let develop_run_id = Box::pin(terminal_run_with_slug("develop")).await;
    let nightly_run_id = Box::pin(terminal_run_with_slug("nightly")).await;

    // Worker with inspects=["develop"] reaching the server over the wire.
    let worker_token = issue_test_inspects_worker_token(&RunId::new(), &["develop".to_string()]);
    let target: ServerTarget = format!("http://{addr}")
        .parse()
        .expect("test server target should parse");
    let api_client = ServerClient::builder()
        .target(target)
        .credential(ClientCredential::Worker(worker_token))
        .connect()
        .await
        .expect("worker client should connect");
    let backend = std::sync::Arc::new(fabro_tool::fabro_client::ClientBackend::new(
        std::sync::Arc::new(api_client),
    ));

    // Probe A: a user JWT through the same socket.
    let user_client = ServerClient::builder()
        .target(format!("http://{addr}").parse::<ServerTarget>().unwrap())
        .credential(ClientCredential::DevToken(issue_test_user_jwt()))
        .connect()
        .await
        .expect("user client should connect");
    user_client
        .resolve_run(&develop_run_id.to_string())
        .await
        .expect("PROBE A: user jwt resolve should work");

    // Worker credential sanity on the wire before the ask chain.
    fabro_tool::FabroToolBackend::resolve_run(backend.as_ref(), &develop_run_id.to_string())
        .await
        .expect("worker resolve should succeed");

    // (a) In-scope ask: the full chain runs — session create over HTTP,
    // turn streamed over SSE, analyst answer returned to the caller.
    let result = ask_run(
        std::sync::Arc::clone(&backend) as std::sync::Arc<dyn fabro_tool::FabroToolBackend>,
        ValidatedAsk::try_from(FabroAskParams {
            run_id:   develop_run_id.to_string(),
            question: "Why did the gate time out?".to_string(),
        })
        .unwrap(),
        &["develop".to_string()],
    )
    .await
    .expect("in-scope ask should succeed through the real wire");
    assert_eq!(result.answer, "The gate timed out waiting for approval.");
    // The strict mock carried every sentinel (positive and negative); a
    // hit proves the wire request satisfied all of them.
    assert_eq!(llm_post.calls(), 1, "exactly one analyst turn");

    // (b) Out-of-scope target: the tool rejects before the round trip.
    let err = ask_run(
        std::sync::Arc::clone(&backend) as std::sync::Arc<dyn fabro_tool::FabroToolBackend>,
        ValidatedAsk::try_from(FabroAskParams {
            run_id:   nightly_run_id.to_string(),
            question: "Why did this run fail?".to_string(),
        })
        .unwrap(),
        &["develop".to_string()],
    )
    .await
    .expect_err("out-of-scope targets must be rejected");
    assert!(err.as_str().contains("inspects scope"), "{err}");

    // (c) Wire-level authz: session creation on the out-of-scope run is
    // rejected BY THE SERVER (403), bypassing the tool-side check.
    let direct_client = ServerClient::builder()
        .target(
            format!("http://{addr}")
                .parse::<ServerTarget>()
                .expect("target should parse"),
        )
        .credential(ClientCredential::Worker(issue_test_inspects_worker_token(
            &RunId::new(),
            &["develop".to_string()],
        )))
        .connect()
        .await
        .expect("direct client should connect");
    let rejected = direct_client
        .create_run_session(nightly_run_id, fabro_api::types::CreateRunSessionRequest {
            title:    Some("revisor question".to_string()),
            model:    None,
            provider: None,
        })
        .await
        .expect_err("server must reject out-of-scope session creation");
    assert!(
        rejected.to_string().contains("Access denied"),
        "expected the server's 403 body on the wire, got: {rejected}"
    );

    server_task.abort();
}

#[tokio::test]
async fn terminal_run_ask_fabro_turns_succeed_hermetically() {
    // Hermetic LLM: point the builtin openai provider at a local SSE mock
    // so the analyst turn runs end to end without network access.
    let mock_llm = httpmock::MockServer::start();
    mock_llm.mock(|when, then| {
        when.method(httpmock::Method::POST);
        then.status(200)
            .header("content-type", "text/event-stream")
            .body(
                // Responses-API SSE: one text delta, then a completed
                // response carrying usage.
                [
                    r#"data: {"type":"response.output_text.delta","delta":"Post-mortem done."}"#,
                    "\n\n",
                    r#"data: {"type":"response.completed","response":{"id":"resp_1","status":"completed","output":[],"usage":{"input_tokens":10,"output_tokens":1,"total_tokens":11}}}"#,
                    "\n\n",
                ]
                .concat(),
            );
    });

    let state = crate::test_support::jwt_auth_state_with_openai_base_url(
        &mock_llm.base_url(),
        TEST_SESSION_SECRET,
    );
    let app = build_router(Arc::clone(&state), jwt_auth_mode());

    // build_agent_session reaches self_server_target() for the analyst's
    // API client; the test state needs the daemon record for that.
    let runtime_directory =
        fabro_config::Storage::new(state.server_storage_dir()).runtime_directory();
    ServerDaemon::new(
        std::process::id(),
        Bind::Tcp("127.0.0.1:32299".parse::<std::net::SocketAddr>().unwrap()),
        runtime_directory.log_path(),
    )
    .write(&runtime_directory)
    .unwrap();
    let run_id = RunId::new();
    let run_store = state.stores.runs.create_run(&run_id).await.unwrap();

    // A finished run whose sandbox instance is a local provider: the
    // terminal-run session build path runs (sandbox reconnect +
    // activation + guard evaluation) and fails cleanly on the missing
    // LLM configuration instead of wedging the session runtime.
    let mut events = vec![workflow_event::Event::RunCreated {
        run_id,
        title: None,
        settings: serde_json::to_value(fabro_types::WorkflowSettings::default()).unwrap(),
        graph: serde_json::to_value(Graph::new("ask-terminal")).unwrap(),
        workflow_source: None,
        labels: std::collections::BTreeMap::default(),
        source_directory: None,
        workflow_slug: Some("develop".to_string()),
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
    }];
    // The Host provider derives a local sandbox id from the canonical
    // directory and attaches only to directories that exist, so the
    // hand-written record uses a real temp dir.
    let workdir = tempfile::tempdir().unwrap();
    let workdir_path = workdir.path().to_path_buf();
    events.push(workflow_event::Event::SandboxInitialized {
        provider:          SandboxProviderKind::LOCAL,
        id:                fabro_sandbox::test_support::local_sandbox_id(&workdir_path).await,
        working_directory: workdir_path.to_string_lossy().to_string(),
        image:             None,
        snapshot:          None,
        repo_cloned:       None,
        clone_origin_url:  None,
        clone_branch:      None,
        workspace_root:    None,
        repos_root:        None,
        primary_repo_path: None,
        primary_repo_link: None,
    });
    events.push(workflow_event::Event::RunPending {
        reason: fabro_types::PendingReason::ApprovalRequired,
        actor:  None,
    });
    events.push(workflow_event::Event::RunRunnable {
        source: fabro_types::RunRunnableSource::StartRequested,
        actor:  None,
    });
    events.push(workflow_event::Event::RunStarting);
    events.push(workflow_event::Event::RunRunning);
    events.push(workflow_event::Event::WorkflowRunCompleted {
        timing:               fabro_types::RunTiming::default(),
        artifact_count:       0,
        status:               "succeeded".to_string(),
        reason:               fabro_types::SuccessReason::Completed,
        failure:              None,
        final_git_commit_sha: None,
        final_patch:          None,
        diff_summary:         None,
        usage:                None,
    });
    for event in events {
        workflow_event::append_event(&run_store, &run_id, &event)
            .await
            .unwrap();
    }

    let user_jwt = issue_test_user_jwt();
    let create_session = |bearer: String| {
        app.clone().oneshot(
            Request::builder()
                .method("POST")
                .uri(api(&format!("/runs/{run_id}/sessions")))
                .header(header::AUTHORIZATION, format!("Bearer {bearer}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&serde_json::json!({ "title": "post-mortem" })).unwrap(),
                ))
                .unwrap(),
        )
    };
    let response = create_session(user_jwt.clone()).await.unwrap();
    let body = response_json!(response, StatusCode::CREATED).await;
    let session_id = body["id"].as_str().unwrap().to_string();

    for turn in 1..=2 {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(api(&format!("/sessions/{session_id}/turns")))
                    .header(header::AUTHORIZATION, format!("Bearer {user_jwt}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "input": format!("why did run {turn} finish?")
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        // Both turns must succeed end to end on a terminal run. This
        // proves the terminal-run session BUILD path (guard evaluation
        // included) works and never wedges the runtime; the local
        // provider reports AlreadyActive, so the eviction+stop wiring
        // (turn-scoped liveness, fabro-b5bd) is proven by the unit test
        // and the wire proof rides on fabro-3b6d's rig.
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let stream = String::from_utf8_lossy(&bytes);
        assert!(
            stream.contains("run.session.turn.succeeded"),
            "turn {turn} should succeed, stream tail: {}",
            &stream[stream.len().saturating_sub(500)..]
        );
        assert!(
            stream.contains("Post-mortem done."),
            "turn {turn} should carry the analyst answer, stream: {stream}"
        );
    }
}

#[tokio::test]
async fn inspects_worker_lists_runs_only_through_declared_workflow_filter() {
    let (state, app) = jwt_auth_app();
    let parent_run_id = unique_run_id();
    let develop_run_id = unique_run_id();
    create_run_with_workflow_slug(&state, develop_run_id, "develop").await;
    // Own child of the worker's run (fabro-95e8 shape: the duplicate-child
    // guard enumerates exactly this slice).
    let child_run_id = unique_run_id();
    let child_store = state.stores.runs.create_run(&child_run_id).await.unwrap();
    workflow_event::append_event(
        &child_store,
        &child_run_id,
        &workflow_event::Event::RunCreated {
            run_id:              child_run_id,
            title:               None,
            settings:            serde_json::to_value(fabro_types::WorkflowSettings::default())
                .unwrap(),
            graph:               serde_json::to_value(Graph::new("merge-upstream")).unwrap(),
            workflow_source:     None,
            labels:              std::collections::BTreeMap::default(),
            source_directory:    None,
            workflow_slug:       Some("merge-upstream".to_string()),
            workflow_version_id: None,
            target:              None,
            automation:          None,
            provenance:          test_support::test_run_provenance(),
            spec_blob:           None,
            git:                 None,
            fork_source_ref:     None,
            retried_from:        None,
            parent_id:           Some(parent_run_id),
            web_url:             None,
        },
    )
    .await
    .unwrap();
    let inspects_token = issue_test_inspects_worker_token(&parent_run_id, &["develop".to_string()]);
    let plain_run_tools_token = issue_test_run_tools_worker_token(&parent_run_id);

    // Unfiltered enumeration is denied for inspects workers.
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            "/runs",
            &inspects_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    // Foreign workflow filter is denied.
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            "/runs?workflow=nightly",
            &inspects_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    // Declared workflow filter succeeds and returns only that workflow.
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            "/runs?workflow=develop",
            &inspects_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let slugs: Vec<&str> = body["data"]
        .as_array()
        .expect("run list data should be an array")
        .iter()
        .filter_map(|run| run["workflow"]["slug"].as_str())
        .collect();
    assert!(!slugs.is_empty(), "the develop run should be listed");
    assert!(slugs.iter().all(|slug| *slug == "develop"), "{slugs:?}");

    // Workers without inspects keep their legacy unfiltered access.
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            "/runs",
            &plain_run_tools_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    // fabro-95e8: own-children enumeration passes the inspects gate — the
    // worker lists parent_id equal to its own run id and sees exactly its
    // children (the enumeration analog of the ADR-0011 created-runs and
    // descendant allowances).
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs?parent_id={parent_run_id}"),
            &inspects_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let child_ids: Vec<&str> = body["data"]
        .as_array()
        .expect("own-children data should be an array")
        .iter()
        .filter_map(|run| run["id"].as_str())
        .collect();
    assert_eq!(
        child_ids,
        vec![child_run_id.to_string().as_str()],
        "own-children enumeration must return exactly the linked child"
    );

    // A parent_id the worker does not own keeps the gate: no widening to
    // arbitrary parent-scoped enumeration.
    let foreign_parent_id = unique_run_id();
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs?parent_id={foreign_parent_id}"),
            &inspects_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;
}

#[tokio::test]
async fn run_tool_worker_ancestor_allowance_independent_of_creator() {
    // fabro-4556 rule 3 in isolation: a run the worker did NOT create
    // becomes targetable when a USER links it below the worker's run.
    // Without the ancestry walk this test fails — creator alone must not
    // carry it.
    let (state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let worker_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let user_child = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_run_tools_worker_token(&worker_run_id);

    // Before the link: the user-created child is foreign to the worker.
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{user_child}/state"),
            &worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    // A user (not the worker) links the child under the worker's run.
    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::PUT,
            &format!("/runs/{user_child}/parent"),
            &user_jwt,
            &json!({ "parent_id": worker_run_id.to_string() }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{user_child}/state"),
            &worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    // Sanity: the allowance came from ancestry, not provenance.
    let cached = state
        .stores
        .runs
        .load_run_projection(&user_child)
        .await
        .unwrap()
        .expect("child should have a projection");
    assert!(
        !matches!(
            &cached.spec.provenance.subject,
            Principal::Worker { run_id } if run_id == &worker_run_id
        ),
        "child must not be worker-created for this test to pin ancestry"
    );
}

#[tokio::test]
async fn run_tool_worker_cyclic_parent_chain_fails_closed() {
    // Cycle guard: a corrupt parent chain (mutual parents, only writable
    // at the store level — the link API rejects cycles) must deny, not
    // loop.
    let (state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let worker_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let run_a = create_run_with_bearer(&app, &user_jwt).await;
    let run_b = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_run_tools_worker_token(&worker_run_id);

    // Store-level corruption: a <-> b mutual parents.
    append_parent_linked_event(&state, run_a, run_b).await;
    append_parent_linked_event(&state, run_b, run_a).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{run_a}/state"),
            &worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;
}

#[tokio::test]
async fn run_tools_worker_cannot_call_user_only_non_mcp_routes() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let origin_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let target_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_run_tools_worker_token(&origin_run_id);

    for (method, path) in [
        (Method::POST, format!("/runs/{target_run_id}/approve")),
        (Method::POST, format!("/runs/{target_run_id}/deny")),
        (Method::GET, format!("/runs/{target_run_id}/timeline")),
    ] {
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
            "{method} {path} unexpectedly accepted run-tools worker token with status {}",
            response.status()
        );
    }
}

#[tokio::test]
async fn stage_artifacts_multipart_round_trip() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id = create_run(&app, MINIMAL_DOT).await;
    let stage_id = "code@2";
    let source_bytes = b"fn main() {}\n";
    let log_bytes = b"build ok\n";
    let manifest = serde_json::json!({
        "entries": [
            {
                "part": "file1",
                "path": "src/lib.rs",
                "sha256": hex::encode(Sha256::digest(source_bytes)),
                "expected_bytes": source_bytes.len(),
                "content_type": "text/plain"
            },
            {
                "part": "file2",
                "path": "logs/output.txt",
                "sha256": hex::encode(Sha256::digest(log_bytes)),
                "expected_bytes": log_bytes.len(),
                "content_type": "text/plain"
            }
        ]
    });
    let boundary = "fabro-test-boundary";

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!(
            "/runs/{run_id}/stages/{stage_id}/artifacts?retry=1"
        )))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(multipart_body(boundary, &manifest, &[
            ("file1", "src/lib.rs", source_bytes),
            ("file2", "logs/output.txt", log_bytes),
        ]))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NO_CONTENT).await;

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}/stages/{stage_id}/artifacts")))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["data"][0]["filename"], "logs/output.txt");
    assert_eq!(body["data"][0]["retry"], 1);
    assert_eq!(body["data"][0]["size"], log_bytes.len());
    assert_eq!(body["data"][1]["filename"], "src/lib.rs");
    assert_eq!(body["data"][1]["retry"], 1);
    assert_eq!(body["data"][1]["size"], source_bytes.len());

    let req = Request::builder()
        .method("GET")
        .uri(api(&format!(
            "/runs/{run_id}/stages/{stage_id}/artifacts/download?filename=logs/output.txt&retry=1"
        )))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let bytes = response_bytes!(response, StatusCode::OK).await;
    assert_eq!(&bytes[..], log_bytes);
}

#[tokio::test]
async fn stage_artifacts_multipart_requires_manifest_first() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_id = create_run(&app, MINIMAL_DOT).await;
    let boundary = "fabro-test-boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file1\"; filename=\"src/lib.rs\"\r\n\r\nfn main() {{}}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"manifest\"\r\nContent-Type: application/json\r\n\r\n{{\"entries\":[{{\"part\":\"file1\",\"path\":\"src/lib.rs\"}}]}}\r\n--{boundary}--\r\n"
    );

    let req = Request::builder()
        .method("POST")
        .uri(api(&format!(
            "/runs/{run_id}/stages/code@2/artifacts?retry=1"
        )))
        .header(
            "content-type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(Body::from(body))
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::BAD_REQUEST).await;
}

#[tokio::test]
async fn create_run_accepts_explicit_title() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let mut intent = test_intent(&app, MINIMAL_DOT).await;
    intent["title"] = json!("  Explicit server title  ");

    let req = Request::builder()
        .method("POST")
        .uri(api("/runs"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_string(&intent).unwrap()))
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::CREATED).await;
    assert_eq!(body["title"], "Explicit server title");

    let run_id = body["id"].as_str().unwrap();
    let detail_response = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(api(&format!("/runs/{run_id}")))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let detail = response_json!(detail_response, StatusCode::OK).await;
    assert_eq!(detail["title"], "Explicit server title");
}

#[tokio::test]
async fn create_run_rejects_invalid_titles() {
    let app = test_app_with();
    for title in [
        "   ".to_string(),
        "First\nSecond".to_string(),
        "x".repeat(101),
    ] {
        let mut intent = test_intent(&app, MINIMAL_DOT).await;
        intent["title"] = json!(title);
        let req = Request::builder()
            .method("POST")
            .uri(api("/runs"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&intent).unwrap()))
            .unwrap();

        let response = app.clone().oneshot(req).await.unwrap();
        assert_status!(response, StatusCode::UNPROCESSABLE_ENTITY).await;
    }
}

#[tokio::test]
async fn patch_run_title_updates_active_and_archived_runs() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();

    let patch_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(api(&format!("/runs/{run_id}")))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "title": "  Active title  " }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let patch_body = response_json!(patch_response, StatusCode::OK).await;
    assert_eq!(patch_body["title"], "Active title");

    let run_store = state.stores.runs.open_run_reader(&run_id).await.unwrap();
    let event_count = run_store.list_events().await.unwrap().len();
    let same_title_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(api(&format!("/runs/{run_id}")))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "title": "Active title" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let same_title_body = response_json!(same_title_response, StatusCode::OK).await;
    assert_eq!(same_title_body["title"], "Active title");
    assert_eq!(
        state
            .stores
            .runs
            .open_run_reader(&run_id)
            .await
            .unwrap()
            .list_events()
            .await
            .unwrap()
            .len(),
        event_count,
        "same-title PATCH should not append an event"
    );

    let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
    for event in [
        workflow_event::Event::RunRunnable {
            source: fabro_types::RunRunnableSource::StartRequested,
            actor:  None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ] {
        workflow_event::append_event(&run_store, &run_id, &event)
            .await
            .unwrap();
    }
    workflow_event::append_event(
        &run_store,
        &run_id,
        &workflow_event::Event::WorkflowRunCompleted {
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
    )
    .await
    .unwrap();
    response_json!(
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(api(&format!("/runs/{run_id}/archive")))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
        StatusCode::OK
    )
    .await;

    let archived_patch_response = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(api(&format!("/runs/{run_id}")))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "title": "Archived title" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let archived_patch_body = response_json!(archived_patch_response, StatusCode::OK).await;
    assert_eq!(archived_patch_body["title"], "Archived title");
    assert!(run_json_archived(&archived_patch_body));
}

#[tokio::test]
async fn patch_run_title_rejects_invalid_titles() {
    let app = test_app_with();
    let run_id = create_run(&app, MINIMAL_DOT).await;

    for title in [String::new(), "Bad\rTitle".to_string(), "x".repeat(101)] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(api(&format!("/runs/{run_id}")))
                    .header("content-type", "application/json")
                    .body(Body::from(json!({ "title": title }).to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_status!(response, StatusCode::BAD_REQUEST).await;
    }
}

#[tokio::test]
async fn list_runs_returns_started_run() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    // List should be empty initially
    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .body(Body::empty())
        .unwrap();

    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["data"].as_array().unwrap().len(), 0);
    assert_eq!(body["meta"]["has_more"].as_bool(), Some(false));

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

    // List should now contain one run
    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .body(Body::empty())
        .unwrap();

    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let items = body["data"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(run_json_id(&items[0]).unwrap(), run_id.to_string());
    assert!(items[0]["goal"].is_string());
    assert!(items[0]["title"].is_string());
    assert!(items[0]["repository"]["name"].is_string());
    assert!(items[0]["timestamps"]["created_at"].is_string());
    assert!(run_json_status(&items[0]).is_object());
    assert!(items[0]["labels"].is_object());
    assert!(run_json_pending_control(&items[0]).is_null());
    assert_eq!(items[0]["usage"]["tokens"]["input"], 0);
    assert!(items[0]["usage"].get("cost").is_none());
}

#[tokio::test]
async fn list_runs_returns_run_list_items() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_and_start_run(&app, MINIMAL_DOT).await;

    {
        let id = run_id.parse::<RunId>().unwrap();
        let mut runs = state.runs.lock().expect("runs lock poisoned");
        let managed_run = runs.get_mut(&id).expect("run should exist");
        managed_run.status = RunStatus::Running;
    }

    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let data = body["data"].as_array().expect("data should be array");
    let item = data
        .iter()
        .find(|i| run_json_id(i) == Some(&run_id))
        .expect("run should be in list");
    assert!(item["goal"].is_string());
    assert!(item["title"].is_string());
    assert!(item["repository"].is_object());
    assert!(item["workflow"]["slug"].is_string() || item["workflow"]["slug"].is_null());
    assert!(item["workflow"]["name"].is_string() || item["workflow"]["name"].is_null());
    assert!(item["workflow"]["graph_name"].is_string());
    assert!(item["labels"].is_object());
    assert!(run_json_status(item).is_object());
    assert!(item["timestamps"]["created_at"].is_string());
    assert!(run_json_pending_control(item).is_null());
    assert_eq!(item["usage"]["tokens"]["input"], 0);
    assert!(item["usage"].get("cost").is_none());
}

#[tokio::test]
async fn list_runs_excludes_removing_status_by_default() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = fixtures::RUN_1;

    // A run in Removing status should not appear by default
    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
        workflow_event::Event::RunRemoving,
    ])
    .await;

    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let data = body["data"].as_array().expect("data should be array");
    assert!(
        !data
            .iter()
            .any(|i| run_json_id(i) == Some(&run_id.to_string())),
        "removing run should not appear by default"
    );

    // ?status=removing opts the bucket in.
    let req = Request::builder()
        .method("GET")
        .uri(api("/runs?status=removing"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let data = body["data"].as_array().expect("data should be array");
    assert!(
        data.iter()
            .any(|i| run_json_id(i) == Some(&run_id.to_string())),
        "?status=removing should opt removing runs in"
    );
}

#[tokio::test]
async fn list_runs_excludes_archived_by_default() {
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
        workflow_event::Event::RunArchived { actor: None },
    ])
    .await;

    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let data = body["data"].as_array().expect("data should be array");
    assert!(
        !data
            .iter()
            .any(|i| run_json_id(i) == Some(&run_id.to_string())),
        "archived run should be hidden when include_archived is unset",
    );
}

#[tokio::test]
async fn list_runs_includes_archived_when_flag_set() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let archived_id = fixtures::RUN_1;
    let succeeded_id = fixtures::RUN_2;

    create_durable_run_with_events(&state, archived_id, &[
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
        workflow_event::Event::RunArchived { actor: None },
    ])
    .await;
    create_durable_run_with_events(&state, succeeded_id, &[
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

    let req = Request::builder()
        .method("GET")
        .uri(api("/runs?include_archived=true"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let data = body["data"].as_array().expect("data should be array");

    let archived_item = data
        .iter()
        .find(|i| run_json_id(i) == Some(&archived_id.to_string()))
        .expect("archived run should appear when include_archived=true");
    assert!(run_json_archived(archived_item));
    assert_eq!(
        run_json_status(archived_item)["kind"].as_str().unwrap(),
        "succeeded"
    );

    let succeeded_item = data
        .iter()
        .find(|i| run_json_id(i) == Some(&succeeded_id.to_string()))
        .expect("non-archived run should still appear");
    assert_eq!(
        run_json_status(succeeded_item)["kind"].as_str().unwrap(),
        "succeeded"
    );
}

#[tokio::test]
async fn get_run_exposes_canonical_operator_statuses() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let succeeded_id = fixtures::RUN_1;
    let removing_id = fixtures::RUN_2;
    let blocked_id = fixtures::RUN_3;

    create_durable_run_with_events(&state, succeeded_id, &[
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

    create_durable_run_with_events(&state, removing_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
        workflow_event::Event::RunRemoving,
    ])
    .await;
    create_durable_run_with_events(&state, blocked_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ])
    .await;
    append_raw_run_event(
        &state,
        blocked_id,
        "status-blocked",
        "2026-04-19T12:00:00Z",
        "run.blocked",
        json!({ "blocked_reason": "human_input_required" }),
        None,
    )
    .await;

    for (run_id, expected_status) in [
        (succeeded_id, "succeeded"),
        (removing_id, "removing"),
        (blocked_id, "blocked"),
    ] {
        let req = Request::builder()
            .method("GET")
            .uri(api(&format!("/runs/{run_id}")))
            .body(Body::empty())
            .unwrap();
        let response = app.clone().oneshot(req).await.unwrap();
        let body = response_json!(response, StatusCode::OK).await;
        assert_eq!(
            run_json_status(&body)["kind"].as_str(),
            Some(expected_status)
        );
    }
}

#[tokio::test]
async fn list_runs_preserves_underlying_run_status_payloads() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let paused_id = fixtures::RUN_1;
    let succeeded_id = fixtures::RUN_2;
    let blocked_id = fixtures::RUN_3;

    create_durable_run_with_events(&state, paused_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
        workflow_event::Event::RunPaused,
    ])
    .await;
    create_durable_run_with_events(&state, succeeded_id, &[
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
    create_durable_run_with_events(&state, blocked_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ])
    .await;
    append_raw_run_event(
        &state,
        blocked_id,
        "blocked-question-1",
        "2026-04-19T12:00:00Z",
        "interview.started",
        json!({
            "question_id": "q-older",
            "question": "Older unresolved question?",
            "stage": "gate",
            "question_type": "multiple_choice",
            "options": [],
            "allow_freeform": false,
            "context_display": null,
            "timeout_seconds": null,
        }),
        Some("gate"),
    )
    .await;
    append_raw_run_event(
        &state,
        blocked_id,
        "blocked-question-2",
        "2026-04-19T12:00:01Z",
        "interview.started",
        json!({
            "question_id": "q-newer",
            "question": "Newer unresolved question?",
            "stage": "gate",
            "question_type": "multiple_choice",
            "options": [],
            "allow_freeform": false,
            "context_display": null,
            "timeout_seconds": null,
        }),
        Some("gate"),
    )
    .await;
    append_raw_run_event(
        &state,
        blocked_id,
        "blocked-status",
        "2026-04-19T12:00:02Z",
        "run.blocked",
        json!({ "blocked_reason": "human_input_required" }),
        None,
    )
    .await;

    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = body_json(response.into_body()).await;
    let data = body["data"].as_array().expect("data should be array");

    let paused_item = data
        .iter()
        .find(|i| run_json_id(i) == Some(&paused_id.to_string()))
        .expect("paused run should be on board");
    assert_eq!(
        run_json_status(paused_item)["kind"].as_str().unwrap(),
        "paused"
    );
    assert!(run_json_status(paused_item)["prior_block"].is_null());

    let succeeded_item = data
        .iter()
        .find(|i| run_json_id(i) == Some(&succeeded_id.to_string()))
        .expect("succeeded run should be on board");
    assert_eq!(
        run_json_status(succeeded_item)["kind"].as_str().unwrap(),
        "succeeded"
    );
    assert_eq!(
        run_json_status(succeeded_item)["reason"].as_str().unwrap(),
        "completed"
    );

    let blocked_item = data
        .iter()
        .find(|i| run_json_id(i) == Some(&blocked_id.to_string()))
        .expect("blocked run should be on board");
    assert_eq!(
        run_json_status(blocked_item)["kind"].as_str().unwrap(),
        "blocked"
    );
    assert_eq!(
        run_json_status(blocked_item)["blocked_reason"]
            .as_str()
            .unwrap(),
        "human_input_required"
    );
    assert!(
        blocked_item["current_question"].is_object(),
        "blocked item should include the current question"
    );
}

#[tokio::test]
async fn list_runs_includes_live_metadata_from_run_state() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = create_and_start_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();
    let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
    for event in [
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
        workflow_event::Event::SandboxInitialized {
            provider:          SandboxProviderKind::LOCAL,
            id:                "sb-test".to_string(),
            working_directory: "/sandbox/workdir".to_string(),
            image:             None,
            snapshot:          None,
            repo_cloned:       None,
            clone_origin_url:  None,
            clone_branch:      None,
            workspace_root:    None,
            repos_root:        None,
            primary_repo_path: None,
            primary_repo_link: None,
        },
        workflow_event::Event::PullRequestCreated {
            pr_url:      "https://github.com/acme/repo/pull/42".to_string(),
            pr_number:   42,
            owner:       "acme".to_string(),
            repo:        "repo".to_string(),
            base_branch: "main".to_string(),
            head_branch: "fabro/run".to_string(),
            head_sha:    Some("final-sha".to_string()),
            title:       "Fix board metadata".to_string(),
            draft:       false,
            auto_merge:  None,
        },
        workflow_event::Event::InterviewStarted {
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
    ] {
        workflow_event::append_event(&run_store, &run_id, &event)
            .await
            .unwrap();
    }

    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let data = body["data"].as_array().expect("data should be array");
    let item = data
        .iter()
        .find(|i| run_json_id(i) == Some(&run_id.to_string()))
        .expect("run should be in board");

    assert_eq!(item["pull_request"]["number"].as_u64(), Some(42));
    assert_eq!(item["sandbox"]["kind"].as_str(), Some("ready"));
    assert_eq!(
        item["sandbox"]["instance"]["runtime"]["id"].as_str(),
        Some("sb-test")
    );
    assert_eq!(
        item["sandbox"]["instance"]["runtime"]["working_directory"].as_str(),
        Some("/sandbox/workdir")
    );
    assert!(item["current_question"].is_object());
}

#[tokio::test]
async fn list_runs_page_limit_preserves_metadata_for_paged_items() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let first_run_id = create_and_start_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();
    let second_run_id = create_and_start_run(&app, MINIMAL_DOT)
        .await
        .parse::<RunId>()
        .unwrap();

    for (run_id, sandbox_id) in [(first_run_id, "sb-first"), (second_run_id, "sb-second")] {
        let run_store = state.stores.runs.open_run(&run_id).await.unwrap();
        for event in [
            workflow_event::Event::RunStarting,
            workflow_event::Event::RunRunning,
            workflow_event::Event::SandboxInitialized {
                provider:          SandboxProviderKind::LOCAL,
                id:                sandbox_id.to_string(),
                working_directory: "/sandbox/workdir".to_string(),
                image:             None,
                snapshot:          None,
                repo_cloned:       None,
                clone_origin_url:  None,
                clone_branch:      None,
                workspace_root:    None,
                repos_root:        None,
                primary_repo_path: None,
                primary_repo_link: None,
            },
        ] {
            workflow_event::append_event(&run_store, &run_id, &event)
                .await
                .unwrap();
        }
    }

    let req = Request::builder()
        .method("GET")
        .uri(api("/runs?page[limit]=1"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    assert_eq!(body["meta"]["has_more"].as_bool(), Some(true));

    let data = body["data"].as_array().expect("data should be array");
    assert_eq!(data.len(), 1);

    let item = &data[0];
    let sandbox_id = item["sandbox"]["instance"]["runtime"]["id"]
        .as_str()
        .expect("paged item should still include sandbox metadata");
    assert!(matches!(sandbox_id, "sb-first" | "sb-second"));
}

#[tokio::test]
async fn list_runs_status_filter_accepts_repeated_values() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    // Running run (will map to BoardColumn::Running)
    let running_id = fixtures::RUN_1;
    create_durable_run_with_events(&state, running_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ])
    .await;

    // Succeeded run (BoardColumn::Succeeded)
    let succeeded_id = fixtures::RUN_2;
    create_durable_run_with_events(&state, succeeded_id, &[
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

    // Pending run (BoardColumn::Pending via Submitted)
    let pending_id = fixtures::RUN_3;
    create_durable_run_with_events(&state, pending_id, &[workflow_event::Event::RunSubmitted {
        definition_blob: None,
    }])
    .await;

    // Single value: only running.
    let req = Request::builder()
        .method("GET")
        .uri(api("/runs?status=running"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let ids: Vec<&str> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(run_json_id)
        .collect();
    assert!(ids.contains(&running_id.to_string().as_str()));
    assert!(!ids.contains(&succeeded_id.to_string().as_str()));
    assert!(!ids.contains(&pending_id.to_string().as_str()));

    // Repeated values: running + succeeded.
    let req = Request::builder()
        .method("GET")
        .uri(api("/runs?status=running&status=succeeded"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let ids: Vec<&str> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(run_json_id)
        .collect();
    assert!(ids.contains(&running_id.to_string().as_str()));
    assert!(ids.contains(&succeeded_id.to_string().as_str()));
    assert!(!ids.contains(&pending_id.to_string().as_str()));
}

#[tokio::test]
async fn list_runs_sort_direction_reverses_order_with_stable_tiebreak() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    // All fixtures share timestamp=0; the id-desc tiebreak controls order.
    let ids = [fixtures::RUN_1, fixtures::RUN_2, fixtures::RUN_3];
    for id in &ids {
        create_durable_run_with_events(&state, *id, &[
            workflow_event::Event::RunSubmitted {
                definition_blob: None,
            },
            workflow_event::Event::RunStarting,
            workflow_event::Event::RunRunning,
        ])
        .await;
    }

    // Default (sort=created_at desc): tiebreak puts higher ids first.
    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .body(Body::empty())
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let observed: Vec<String> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(run_json_id)
        .map(str::to_string)
        .collect();
    let mut expected: Vec<String> = ids.iter().map(std::string::ToString::to_string).collect();
    expected.sort_by(|a, b| b.cmp(a)); // desc by id
    assert_eq!(observed, expected, "default desc order with id tiebreak");

    // Ascending: timestamps still tie, then id desc tiebreak still applies.
    let req = Request::builder()
        .method("GET")
        .uri(api("/runs?sort=created_at&direction=asc"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let observed: Vec<String> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(run_json_id)
        .map(str::to_string)
        .collect();
    assert_eq!(
        observed, expected,
        "asc still uses id-desc tiebreak for tied keys"
    );
}

#[tokio::test]
async fn list_runs_sort_by_status_groups_by_bucket() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    // BoardColumn enum order: pending < runnable < initializing < running <
    // blocked < succeeded < failed < archived < removing. Use three distinct
    // buckets.
    let pending_id = fixtures::RUN_1;
    create_durable_run_with_events(&state, pending_id, &[workflow_event::Event::RunSubmitted {
        definition_blob: None,
    }])
    .await;

    let succeeded_id = fixtures::RUN_2;
    create_durable_run_with_events(&state, succeeded_id, &[
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

    let running_id = fixtures::RUN_3;
    create_durable_run_with_events(&state, running_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
    ])
    .await;

    // sort=status asc: pending < running < succeeded.
    let req = Request::builder()
        .method("GET")
        .uri(api("/runs?sort=status&direction=asc"))
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let observed: Vec<String> = body["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(run_json_id)
        .map(str::to_string)
        .collect();
    assert_eq!(observed, vec![
        pending_id.to_string(),
        running_id.to_string(),
        succeeded_id.to_string(),
    ]);
}

#[tokio::test]
async fn run_tools_worker_registers_contents_then_creates_by_version_id() {
    let (state, app) = jwt_auth_app();
    let parent_id = create_run_with_bearer(&app, &issue_test_user_jwt()).await;
    let token = issue_test_run_tools_worker_token(&parent_id);
    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            "/workflow-versions",
            &token,
            &json!({
                "entrypoint": "child.fabro",
                "files": {"child.fabro": MINIMAL_DOT},
                "workflow_dependencies": {}
            }),
        ))
        .await
        .unwrap();
    let registered = response_json!(response, StatusCode::CREATED).await;
    let response = app
        .oneshot(json_bearer_request(
            Method::POST,
            "/runs",
            &token,
            &json!({
                "workflow_version_id": registered["workflow_version_id"],
                "target": {"kind": "none"},
                "args": {"dry_run": true},
                "parent_id": parent_id,
                "goal": "A child created from sandbox-supplied contents"
            }),
        ))
        .await
        .unwrap();
    let child = response_json!(response, StatusCode::CREATED).await;
    assert_eq!(child["parent_id"], parent_id.to_string());
    assert_eq!(child["lifecycle"]["status"]["kind"], "submitted");
    let child_id = child["id"].as_str().unwrap().parse::<RunId>().unwrap();
    let projection = state
        .stores
        .runs
        .load_run_projection(&child_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::to_value(projection.spec.workflow_version_id).unwrap(),
        registered["workflow_version_id"]
    );
    assert_eq!(projection.spec.target, Some(RunTarget::None {}));
    assert!(projection.start.is_none());
}
