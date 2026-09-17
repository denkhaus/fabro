use super::*;

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

/// Posts a Git and a `none` run intent against `state` and asserts both are
/// rejected as `integration_unavailable` without persisting anything.

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
