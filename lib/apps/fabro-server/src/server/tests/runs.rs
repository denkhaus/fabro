use super::*;

#[test]
fn replace_settings_caches_invalid_manifest_run_settings_tolerantly() {
    let state = test_app_state_with_options(
        server_settings_from_toml(
            r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
url = "http://old.example.com"
"#,
        ),
        manifest_run_defaults_from_toml(
            r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
url = "http://old.example.com"
"#,
        ),
        5,
    );

    let updated = r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[server.web]
url = "http://new.example.com"

[run.environment]
id = "missing"
"#;

    state
        .replace_runtime_settings(resolved_runtime_settings_from_toml(updated))
        .expect("invalid run defaults should not block replace");

    assert_eq!(state.canonical_origin().unwrap(), "http://new.example.com");
    assert!(
        state.manifest_run_settings().is_err(),
        "manifest run settings should stay tolerant for invalid defaults"
    );
}

#[test]
fn system_sandbox_provider_defaults_when_manifest_run_settings_do_not_resolve() {
    let (_environment_temp, environment_store) = test_environment_store(None, true);
    let (_mcp_temp, mcp_server_store) = test_mcp_server_store();
    let source = r#"
_version = 1

[run.environment]
id = "missing"
"#;
    let manifest_run_settings = resolve_manifest_run_settings_with_catalog(
        &run_manifest::manifest_run_defaults(Some(&manifest_run_defaults_from_toml(source))),
        &environment_store,
        &mcp_server_store,
    );

    assert_eq!(
        system_sandbox_provider(&manifest_run_settings),
        SandboxProviderKind::default().to_string()
    );
}

#[tokio::test]
async fn delete_secret_by_name_removes_file_secret() {
    let state = TestAppStateBuilder::new().build();
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let create_req = Request::builder()
        .method("POST")
        .uri(api("/secrets"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "/tmp/test.pem",
                "value": "pem-data",
                "type": "file",
            }))
            .unwrap(),
        ))
        .unwrap();
    let create_response = app.clone().oneshot(create_req).await.unwrap();
    assert_status!(create_response, StatusCode::OK).await;

    let delete_req = Request::builder()
        .method("DELETE")
        .uri(api("/secrets"))
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_string(&serde_json::json!({
                "name": "/tmp/test.pem",
            }))
            .unwrap(),
        ))
        .unwrap();

    let delete_response = app.oneshot(delete_req).await.unwrap();
    assert_status!(delete_response, StatusCode::NO_CONTENT).await;
    assert!(state.stores.vault.list().await.unwrap().is_empty());
}

#[test]
fn agent_fabro_tools_enabled_combines_run_flag_and_node_level_opt_in() {
    fn spec_with(flag: bool, node_fabro_tools: Option<&str>) -> RunSpec {
        let mut graph = Graph::new("develop");
        if let Some(fabro_tools) = node_fabro_tools {
            let mut planner = Node::new("planner");
            planner.attrs.insert(
                "fabro_tools".to_string(),
                AttrValue::String(fabro_tools.to_string()),
            );
            graph.nodes.insert("planner".to_string(), planner);
        }
        let mut settings = WorkflowSettings::default();
        settings.run.agent.fabro_tools = flag;
        RunSpec {
            run_id: RunId::new(),
            settings,
            graph,
            graph_source: None,
            workflow_slug: Some("develop".to_string()),
            workflow_version_id: None,
            target: None,
            automation: None,
            source_directory: None,
            git: None,
            labels: HashMap::new(),
            provenance: test_support::test_run_provenance(),
            definition_blob: None,
            spec_blob: None,
            fork_source_ref: None,
        }
    }

    // (a) fabro-c419: node-level fabro_tools with the run flag off still
    // provisions the fabro run tool services for the run.
    assert!(super::agent_fabro_tools_enabled(&spec_with(
        false,
        Some("fabro_runs_list")
    )));
    // (b) Neither the run flag nor a node-level opt-in: no services.
    assert!(!super::agent_fabro_tools_enabled(&spec_with(false, None)));
    // (c) Run-wide flag alone: unchanged, enabled.
    assert!(super::agent_fabro_tools_enabled(&spec_with(true, None)));
}

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
async fn post_runs_run_intent_dry_run_uses_configured_target_provider() {
    let folder = tempfile::tempdir().unwrap();
    let folder_path = folder
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let cases = [
        (
            test_app_state(),
            None,
            json!({
                "kind": "git",
                "repo": "fabro-sh/fabro",
                "branch": "main"
            }),
            json!({ "dry_run": true }),
        ),
        (
            test_app_state(),
            None,
            json!({ "kind": "none" }),
            json!({ "dry_run": true }),
        ),
        (
            TestAppStateBuilder::new()
                .default_environment_provider(Some(SandboxProviderKind::DAYTONA))
                .vault_entries([(fabro_static::EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
                .build(),
            Some("_version = 1\n[run.execution]\nmode = \"dry_run\"\n"),
            json!({ "kind": "none" }),
            json!({}),
        ),
        (
            TestAppStateBuilder::new()
                .default_environment_provider(Some(SandboxProviderKind::DAYTONA))
                .vault_entries([(fabro_static::EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
                .build(),
            Some("_version = 1\n[run.execution]\nmode = \"dry_run\"\n"),
            json!({
                "kind": "git",
                "repo": "fabro-sh/fabro",
                "branch": "main"
            }),
            json!({}),
        ),
        (
            TestAppStateBuilder::new()
                .runtime_settings(
                    default_test_server_settings(),
                    manifest_run_defaults_from_toml("[run.execution]\nmode = \"dry_run\"\n"),
                )
                .default_environment_provider(Some(SandboxProviderKind::LOCAL))
                .vault_entries([(fabro_static::EnvVars::OPENAI_API_KEY, "test-openai-api-key")])
                .build(),
            None,
            json!({ "kind": "folder", "path": folder_path }),
            json!({}),
        ),
    ];

    for (state, workflow_toml, target, args) in cases {
        let app = crate::test_support::build_test_router(Arc::clone(&state));
        let workflow_version_id = store_workflow_version(&state, MINIMAL_DOT, workflow_toml).await;
        let body = post_run_intent(
            &app,
            json!({
                "workflow_version_id": workflow_version_id,
                "target": target,
                "args": args
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

        assert_eq!(projection.spec.settings.run.execution.mode, RunMode::DryRun);
        assert_eq!(
            serde_json::to_value(projection.spec.target.unwrap()).unwrap(),
            target
        );
    }
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

#[tokio::test]
async fn run_usage_includes_live_stage_timing_in_rows_and_totals() {
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

    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
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
    let row_timing = &stages[0]["timing"];
    assert!(row_timing["active_time_ms"].as_u64().unwrap() > 0);
    assert_eq!(row_timing["tool_time_ms"], row_timing["active_time_ms"]);
    assert_eq!(&body["totals"]["timing"], row_timing);
}

/// `checkpoint.completed_nodes` records every visit, so a looped node appears
/// once per re-entry. Usage must dedup so a retried node renders as one row
/// and `runtime_secs` is summed across all visits exactly once.

#[tokio::test]
async fn run_usage_dedups_retried_nodes_and_sums_their_durations() {
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

    // Visit 1 of `verify` — completed in 1.5s.
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

    // Visit 2 of `verify` — completed in 0.8s.
    append_scoped_stage_event(
        &state,
        run_id,
        "verify",
        2,
        &workflow_event::Event::StageCompleted {
            node_id: "verify".to_string(),
            name: "Verify".to_string(),
            index: 1,
            timing: fabro_types::StageTiming::wall_only(800),
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

    // Checkpoint records `verify` twice (once per visit) — this is what makes
    // the dedup necessary.
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
            node_retries: std::collections::BTreeMap::new(),
            context_values: std::collections::BTreeMap::new(),
            node_outcomes: std::collections::BTreeMap::from([(
                "verify".to_string(),
                Outcome::default(),
            )]),
            next_node_id: Some("done".to_string()),
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
        .clone()
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
    assert_eq!(
        stages.len(),
        1,
        "expected one row for the retried verify node"
    );
    assert_eq!(stages[0]["stage"]["id"], "verify");
    // Duration on the row is the sum across visits (1.5s + 0.8s = 2.3s).
    assert!(
        stages[0]["timing"]["wall_time_ms"].as_u64().unwrap() == 2300,
        "row runtime_secs should sum visits, got {}",
        stages[0]["timing"]["wall_time_ms"]
    );

    // Totals must not double-count: a single 2.3s, not 4.6s.
    assert!(
        body["totals"]["timing"]["wall_time_ms"].as_u64().unwrap() == 2300,
        "totals.runtime_secs should sum visits exactly once, got {}",
        body["totals"]["timing"]["wall_time_ms"]
    );
}

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
async fn run_usage_retried_node_then_succeeded_emits_one_row_with_final_attempt_duration() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();

    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
        workflow_event::Event::StageStarted {
            graph_visit:           None,
            resumed_from_stage_id: None,
            node_id:               "work".to_string(),
            name:                  "Work".to_string(),
            index:                 0,
            handler_type:          "command".to_string(),
            attempt:               1,
            max_attempts:          3,
        },
        workflow_event::Event::StageFailed {
            node_id:        "work".to_string(),
            name:           "Work".to_string(),
            index:          0,
            failure:        FailureDetail::new("transient", FailureCategory::TransientInfra),
            will_retry:     true,
            timing:         fabro_types::StageTiming::wall_only(10),
            usage_by_model: Vec::new(),
            usage:          None,
            actor:          None,
        },
        workflow_event::Event::StageRetrying {
            node_id:      "work".to_string(),
            name:         "Work".to_string(),
            index:        0,
            attempt:      2,
            max_attempts: 3,
            delay_ms:     0,
        },
        workflow_event::Event::StageStarted {
            graph_visit:           None,
            resumed_from_stage_id: None,
            node_id:               "work".to_string(),
            name:                  "Work".to_string(),
            index:                 0,
            handler_type:          "command".to_string(),
            attempt:               2,
            max_attempts:          3,
        },
        workflow_event::Event::StageCompleted {
            node_id: "work".to_string(),
            name: "Work".to_string(),
            index: 0,
            timing: fabro_types::StageTiming::wall_only(25),
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
            attempt: 2,
            max_attempts: 3,
        },
    ])
    .await;

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
    assert_eq!(stages.len(), 1, "retry collapses to one row per node_id");
    let row = &stages[0];
    assert_eq!(row["stage"]["id"], "work");
    assert_eq!(
        row["state"], "succeeded",
        "final state mirrors the latest StageCompleted"
    );
    let runtime = row["timing"]["wall_time_ms"].as_u64().unwrap();
    assert_eq!(
        runtime, 25,
        "runtime should equal final attempt's 25ms, got {runtime}"
    );
}

#[tokio::test]
async fn run_usage_revisited_node_collapses_to_two_rows_with_summed_visit_duration() {
    let state = test_app_state_with_isolated_storage();
    let app = crate::test_support::build_test_router(Arc::clone(&state));
    let run_id = RunId::new();

    create_durable_run_with_events(&state, run_id, &[
        workflow_event::Event::RunSubmitted {
            definition_blob: None,
        },
        workflow_event::Event::RunStarting,
        workflow_event::Event::RunRunning,
        // A → B → A loop. Per-visit `node_visits` payload steers the reducer
        // to attribute each StageCompleted to the right visit.
        revisit_test_started("a"),
        revisit_test_completed_with_visit("a", 1, 1),
        revisit_test_started("b"),
        revisit_test_completed_with_visit("b", 2, 1),
        revisit_test_started("a"),
        revisit_test_completed_with_visit("a", 99, 2),
    ])
    .await;

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
    assert_eq!(stages.len(), 2, "two distinct node_ids → two rows");
    assert_eq!(
        stages[0]["stage"]["id"], "a",
        "A appeared first → A's row first"
    );
    assert_eq!(stages[1]["stage"]["id"], "b");
    let a_runtime = stages[0]["timing"]["wall_time_ms"].as_u64().unwrap();
    assert_eq!(
        a_runtime, 100,
        "A should sum both visit durations (1ms + 99ms), got {a_runtime}"
    );
    let b_runtime = stages[1]["timing"]["wall_time_ms"].as_u64().unwrap();
    assert_eq!(
        b_runtime, 2,
        "B should carry its single visit's duration (2ms), got {b_runtime}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn worker_principal_run_without_vault_token_takes_no_token_path() {
    let source = r#"
_version = 1

[server.auth]
methods = ["dev-token"]

[run.environment]
id = "local"

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
    // Deliberately NO GITHUB_TOKEN in the vault: a token-requesting User run
    // would hard-fail here, the gated Worker run must not care.
    let app = crate::test_support::build_test_router(Arc::clone(&state));

    let run_settings = r#"
[run.git.author]
name = "Bridge Test"
email = "bridge@test.invalid"

[run.integrations.github.permissions]
contents = "read"
"#;
    let parent_run_id = bridge_intent_run(&app, None, run_settings)
        .await
        .parse::<RunId>()
        .unwrap();
    let worker_token = issue_test_run_tools_worker_token(&parent_run_id);
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
        "worker run with permissions config must succeed without any vault token"
    );
    assert_eq!(
        observations
            .lock()
            .expect("bridge observation lock poisoned")
            .clone(),
        vec![GithubBridgeObservation {
            token_source_present: false,
            env_has_github_token: false,
        }],
        "no credential bridge, no GITHUB_TOKEN in the stage env"
    );
}

/// Build the (state, router, run_id) triple every PR-endpoint test
/// needs. Use this instead of repeating the
/// state/build_router/fixtures::RUN_1 incantation per test.

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

/// Builds an app state over shared object, blob, and summary stores so a test
/// can drop it and open a second state that sees the same durable data.

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
async fn worker_token_accepts_run_scoped_routes_and_falls_back_to_user_jwt() {
    let (state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_worker_token(&run_id);
    let other_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let other_worker_token = issue_test_worker_token(&other_run_id);
    let blob_hash = state
        .stores
        .runs
        .open_run(&run_id)
        .await
        .unwrap()
        .write_blob(b"preloaded blob")
        .await
        .unwrap();

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{run_id}/state"),
            &worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    for path in [
        format!("/runs/{run_id}"),
        format!("/runs/{run_id}/questions"),
    ] {
        let response = app
            .clone()
            .oneshot(bearer_request(
                Method::GET,
                &path,
                &worker_token,
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_status!(response, StatusCode::OK).await;
    }

    let append_body = serde_json::to_vec(&serde_json::json!({
        "id": "evt-run-notice",
        "ts": "2026-04-23T12:00:00Z",
        "event": "run.notice",
        "run_id": run_id.to_string(),
        "properties": {
            "level": "info",
            "code": "worker",
            "message": "hello"
        }
    }))
    .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api(&format!("/runs/{run_id}/events")))
                .header(header::AUTHORIZATION, format!("Bearer {worker_token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(append_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{run_id}/events"),
            &worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::POST,
            &format!("/runs/{run_id}/blobs"),
            &worker_token,
            Body::from("worker blob"),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

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
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{run_id}/state"),
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
            &format!("/runs/{run_id}/state"),
            &other_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;
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

/// fabro-06e0: the live pull request details route is a run-management
/// read, so an inspects-declared worker (develop planner, revisor) can
/// enrich linked PR state for runs of workflows it inspects — without
/// the worker ever holding GitHub credentials.

#[tokio::test]
async fn run_tool_worker_cross_run_routes_require_inspects_scope() {
    // fabro-4556: an `agent:run_tools` worker no longer manages arbitrary
    // runs. Allowed targets: the worker's own run, runs of workflows its
    // token declares in `inspects`, runs the worker created, and runs
    // descended from it. Everything else on run-management routes is 403.
    let (state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let parent_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let foreign_run_id = create_run_with_bearer(&app, &user_jwt).await;
    let run_tool_worker_token = issue_test_run_tools_worker_token(&parent_run_id);

    // Unchanged surfaces: enumeration and resolve stay worker-accessible.
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            "/runs",
            &run_tool_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/resolve?selector={foreign_run_id}"),
            &run_tool_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    // Foreign run: every run-management route denies the worker now.
    for path in [
        format!("/runs/{foreign_run_id}"),
        format!("/runs/{foreign_run_id}/state"),
        format!("/runs/{foreign_run_id}/events"),
        format!("/runs/{foreign_run_id}/questions"),
    ] {
        let response = app
            .clone()
            .oneshot(bearer_request(
                Method::GET,
                &path,
                &run_tool_worker_token,
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_status!(response, StatusCode::FORBIDDEN).await;
    }

    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            &format!("/runs/{foreign_run_id}/start"),
            &run_tool_worker_token,
            &json!({ "resume": false }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::POST,
            &format!("/runs/{foreign_run_id}/cancel"),
            &run_tool_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    for (method, path) in [
        (Method::POST, format!("/runs/{foreign_run_id}/archive")),
        (Method::POST, format!("/runs/{foreign_run_id}/unarchive")),
        (Method::POST, format!("/runs/{foreign_run_id}/interrupt")),
    ] {
        let response = app
            .clone()
            .oneshot(bearer_request(
                method,
                &path,
                &run_tool_worker_token,
                Body::empty(),
            ))
            .await
            .unwrap();
        assert_status!(response, StatusCode::FORBIDDEN).await;
    }

    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            &format!("/runs/{foreign_run_id}/steer"),
            &run_tool_worker_token,
            &json!({ "text": "continue", "interrupt": false }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::POST,
            &format!("/runs/{foreign_run_id}/questions/q-1/answer"),
            &run_tool_worker_token,
            &json!({ "kind": "yes" }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    // Self-access passes without any scope walk.
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{parent_run_id}/state"),
            &run_tool_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    // Creator allowance: a run created BY the worker stays manageable even
    // before any parent link exists — the sub-workflow pattern.
    let created_child = create_run_with_bearer(&app, &run_tool_worker_token).await;
    let projection = state
        .stores
        .runs
        .load_run_projection(&created_child)
        .await
        .unwrap()
        .expect("created run should have a projection");
    assert_eq!(projection.spec.provenance.subject, Principal::Worker {
        run_id: parent_run_id,
    },);

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{created_child}/state"),
            &run_tool_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    // The creator may link itself as the child's parent and unlink again;
    // false parenthood of foreign runs is denied by the same rule.
    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::PUT,
            &format!("/runs/{created_child}/parent"),
            &run_tool_worker_token,
            &json!({ "parent_id": parent_run_id.to_string() }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::DELETE,
            &format!("/runs/{created_child}/parent"),
            &run_tool_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    // Linking a FOREIGN run as parent is denied — that would mint read
    // access out of nothing.
    let response = app
        .clone()
        .oneshot(json_bearer_request(
            Method::PUT,
            &format!("/runs/{foreign_run_id}/parent"),
            &run_tool_worker_token,
            &json!({ "parent_id": parent_run_id.to_string() }),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::FORBIDDEN).await;

    // Unknown run: 404 parity with the user path (not a scope 403).
    let unknown_run_id = RunId::new();
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{unknown_run_id}/state"),
            &run_tool_worker_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;

    // inspects allowance: a worker declaring the target's workflow passes.
    let smoke_run_id = RunId::new();
    create_run_with_workflow_slug(&state, smoke_run_id, "smoke").await;
    let inspects_token = issue_test_inspects_worker_token(&RunId::new(), &["smoke".to_string()]);
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{smoke_run_id}/state"),
            &inspects_token,
            Body::empty(),
        ))
        .await
        .unwrap();
    assert_status!(response, StatusCode::OK).await;

    // ...while the same worker without the declaration is denied.
    let response = app
        .clone()
        .oneshot(bearer_request(
            Method::GET,
            &format!("/runs/{smoke_run_id}/state"),
            &run_tool_worker_token,
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

/// Store-level parent-link corruption for authorization tests: the link
/// API rejects cycles, so a cyclic chain can only be written directly as
/// events.

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
async fn base_worker_token_is_rejected_by_run_tool_only_routes() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_worker_token(&run_id);

    for (method, path) in [
        (Method::GET, "/runs".to_string()),
        (Method::POST, "/runs".to_string()),
        (Method::GET, "/runs/resolve?selector=latest".to_string()),
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
            "{method} {path} unexpectedly accepted base worker token with status {}",
            response.status()
        );
    }
}

#[tokio::test]
async fn run_create_without_agent_session_header_keeps_user_created_by() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(api("/runs"))
                .header(header::AUTHORIZATION, format!("Bearer {user_jwt}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    test_intent_with_bearer(
                        &app,
                        "workflow.fabro",
                        MINIMAL_DOT,
                        None,
                        Some(&user_jwt),
                    )
                    .await
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = response_json!(response, StatusCode::CREATED).await;
    assert_eq!(body["created_by"]["kind"], "user");
    assert_eq!(body["created_by"]["login"], "octocat");
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
async fn batch_delete_requires_user_authentication() {
    let (_state, app) = jwt_auth_app();
    let user_jwt = issue_test_user_jwt();
    let run_id = create_run_with_bearer(&app, &user_jwt).await;
    let worker_token = issue_test_worker_token(&run_id);
    let body = batch_delete_body(&[run_id], false);

    let unauthenticated = app
        .clone()
        .oneshot(json_request(Method::POST, "/runs/delete", &body))
        .await
        .unwrap();
    assert_status!(unauthenticated, StatusCode::UNAUTHORIZED).await;

    let worker_response = app
        .oneshot(json_bearer_request(
            Method::POST,
            "/runs/delete",
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
        "/runs/delete unexpectedly accepted worker token with status {}",
        worker_response.status()
    );
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

#[tokio::test]
async fn demo_list_runs_returns_run_list_items() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(state);
    let req = Request::builder()
        .method("GET")
        .uri(api("/runs"))
        .header("X-Fabro-Demo", "1")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    let data = body["data"].as_array().expect("data should be array");
    assert!(!data.is_empty(), "demo should return runs");
    let first = &data[0];
    assert!(first["id"].is_string());
    assert!(first["goal"].is_string());
    assert!(first["repository"].is_object());
    assert!(first["title"].is_string());
    assert!(run_json_status(first).is_object());
    assert!(first["workflow"]["slug"].is_string() || first["workflow"]["slug"].is_null());
    assert!(first["labels"].is_object());
    assert!(first["timestamps"]["created_at"].is_string());
}

#[tokio::test]
async fn demo_get_run_returns_run_summary_shape() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(state);
    let run_id = RunId::with_timestamp(
        "2026-03-06T14:30:00Z"
            .parse()
            .expect("demo timestamp should parse"),
        1,
    );
    let req = Request::builder()
        .method("GET")
        .uri(api(&format!("/runs/{run_id}")))
        .header("X-Fabro-Demo", "1")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    let body = response_json!(response, StatusCode::OK).await;
    // Should have Run fields, not RunStatusResponse fields
    assert!(body["id"].is_string(), "should have id field");
    assert!(body["goal"].is_string(), "should have goal field");
    assert!(
        body["workflow"]["slug"].is_string(),
        "should have workflow.slug field"
    );
    assert!(body["lifecycle"]["queue_position"].is_null());
}

#[tokio::test]
async fn demo_get_run_returns_404_for_unknown_run() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(state);
    let req = Request::builder()
        .method("GET")
        .uri(api("/runs/nonexistent-run-id"))
        .header("X-Fabro-Demo", "1")
        .body(Body::empty())
        .unwrap();
    let response = app.oneshot(req).await.unwrap();
    assert_status!(response, StatusCode::NOT_FOUND).await;
}

#[tokio::test]
async fn demo_workflows_return_list_detail_and_runs() {
    let state = test_app_state();
    let app = crate::test_support::build_test_router(state);

    let list_req = Request::builder()
        .method("GET")
        .uri(api("/workflows"))
        .header("X-Fabro-Demo", "1")
        .body(Body::empty())
        .unwrap();
    let list_response = app.clone().oneshot(list_req).await.unwrap();
    let list_body = response_json!(list_response, StatusCode::OK).await;
    let workflows = list_body["data"]
        .as_array()
        .expect("workflow list data should be an array");
    assert!(!workflows.is_empty(), "demo should return workflows");
    let first = &workflows[0];
    assert!(first["name"].is_string());
    assert!(first["slug"].is_string());
    assert!(first["filename"].is_string());
    assert!(first["last_run"].is_object() || first["last_run"].is_null());
    assert!(first["schedule"].is_object() || first["schedule"].is_null());

    let detail_req = Request::builder()
        .method("GET")
        .uri(api("/workflows/implement"))
        .header("X-Fabro-Demo", "1")
        .body(Body::empty())
        .unwrap();
    let detail_response = app.clone().oneshot(detail_req).await.unwrap();
    let detail_body = response_json!(detail_response, StatusCode::OK).await;
    assert_eq!(detail_body["slug"], "implement");
    assert!(detail_body["settings"].is_object());
    assert!(
        detail_body["graph"]
            .as_str()
            .is_some_and(|graph| graph.contains("digraph"))
    );

    let runs_req = Request::builder()
        .method("GET")
        .uri(api("/workflows/implement/runs"))
        .header("X-Fabro-Demo", "1")
        .body(Body::empty())
        .unwrap();
    let runs_response = app.oneshot(runs_req).await.unwrap();
    let runs_body = response_json!(runs_response, StatusCode::OK).await;
    let runs = runs_body["data"]
        .as_array()
        .expect("workflow runs data should be an array");
    assert!(
        runs.iter()
            .all(|run| run["workflow"]["slug"].as_str() == Some("implement")),
        "workflow run list should be scoped to the requested workflow"
    );
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
