use super::*;

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
