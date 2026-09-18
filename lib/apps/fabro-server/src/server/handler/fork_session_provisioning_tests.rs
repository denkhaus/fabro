//! FORK-ONLY PRESENCE PIN (fabro-8d30 part b, PR #225): fresh sandbox
//! provisioning for Ask-Fabro turns on terminal runs whose recorded sandbox
//! was reaped. The implementation lives in `handler/sessions.rs`
//! (`provision_replacement_sandbox` + `replacement_sandbox_spec`); this file
//! exists only on our fork so a merge resolution can never silently drop the
//! feature's tests the way the 00ffd60f6 incident dropped the
//! duplicate-child guard. Registry row: `.agents/skills/merge-upstream/
//! references/touchpoints.md` ("Terminal-run sandbox provisioning").

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::http::StatusCode;
    use fabro_test::{TwinScenario, TwinScenarios, twin_openai};
    use fabro_types::{RunId, SessionId, test_support};

    use super::super::sessions;
    use super::super::sessions::resume_tests::{
        MODEL, completed_run, json_response, post_json, turn, twin_backed_state,
    };
    use super::super::sessions::{provision_replacement_sandbox, replacement_sandbox_spec};
    use crate::server::spawn_scheduler;
    use crate::test_support::{TestAppStateBuilder, build_test_router};

    fn replacement_run_spec() -> fabro_types::RunSpec {
        let run_id = RunId::new();
        let mut graph = fabro_types::Graph::new("test");
        for node_id in ["start", "exit"] {
            graph
                .nodes
                .insert(node_id.to_string(), fabro_types::Node::new(node_id));
        }
        fabro_types::RunSpec {
            run_id,
            settings: fabro_types::WorkflowSettings::default(),
            graph,
            graph_source: None,
            workflow_slug: None,
            workflow_version_id: None,
            target: None,
            automation: None,
            source_directory: None,
            labels: HashMap::default(),
            provenance: test_support::test_run_provenance(),
            definition_blob: None,
            spec_blob: None,
            git: None,
            fork_source_ref: None,
        }
    }

    fn recorded_runtime(
        working_directory: &str,
        repo_cloned: Option<bool>,
        origin: Option<&str>,
        branch: Option<&str>,
    ) -> fabro_types::RunSandboxRuntime {
        fabro_types::RunSandboxRuntime {
            id: "recorded-id".to_string(),
            working_directory: working_directory.to_string(),
            repo_cloned,
            clone_origin_url: origin.map(str::to_string),
            clone_branch: branch.map(str::to_string),
            workspace_root: None,
            repos_root: None,
            primary_repo_path: None,
            primary_repo_link: None,
        }
    }

    fn recorded_instance(
        provider: fabro_types::SandboxProviderKind,
        runtime: fabro_types::RunSandboxRuntime,
    ) -> fabro_types::RunSandboxInstance {
        fabro_types::RunSandboxInstance {
            provider,
            image: None,
            snapshot: None,
            runtime,
        }
    }

    /// A local replacement re-designates the recorded working directory and
    /// clones nothing: the Host provider carries no labels, and the run's
    /// directory identity is the sandbox's.
    #[test]
    fn replacement_spec_for_local_reuses_the_recorded_working_directory() {
        let recorded = recorded_instance(
            fabro_types::SandboxProviderKind::LOCAL,
            recorded_runtime("/repos/acme/run", Some(false), None, None),
        );
        let spec = replacement_sandbox_spec(
            RunId::new(),
            &replacement_run_spec(),
            &recorded,
            fabro_sandbox::driver::ProviderAccess::default(),
            None,
        )
        .expect("the local replacement spec builds");

        assert_eq!(spec.provider(), fabro_types::SandboxProviderKind::LOCAL);
        assert_eq!(spec.working_directory(), Some("/repos/acme/run"));
        assert!(spec.clone.skip);
        assert_eq!(spec.run_id, None);
    }

    /// A clone-based replacement carries the run id (the label source the
    /// managed inventory reads) and clones the origin and branch the run's
    /// sandbox record says the original checkout used.
    #[test]
    fn replacement_spec_for_a_cloned_run_carries_run_id_and_recorded_clone() {
        let recorded = recorded_instance(
            fabro_types::SandboxProviderKind::DOCKER,
            recorded_runtime(
                "/workspace/rack-test",
                Some(true),
                Some("https://github.com/brynary/rack-test"),
                Some("main"),
            ),
        );
        let run_id = RunId::new();
        let spec = replacement_sandbox_spec(
            run_id,
            &replacement_run_spec(),
            &recorded,
            fabro_sandbox::driver::ProviderAccess::default(),
            None,
        )
        .expect("the docker replacement spec builds");

        assert_eq!(spec.provider(), fabro_types::SandboxProviderKind::DOCKER);
        assert_eq!(spec.run_id, Some(run_id));
        assert!(!spec.clone.skip);
        assert_eq!(
            spec.clone.origin_url.as_deref(),
            Some("https://github.com/brynary/rack-test")
        );
        assert_eq!(spec.clone.branch.as_deref(), Some("main"));
    }

    /// A record whose run never cloned provisions an empty workspace — the
    /// clone-source contract, not an error.
    #[test]
    fn replacement_spec_without_a_recorded_clone_provisions_an_empty_workspace() {
        let recorded = recorded_instance(
            fabro_types::SandboxProviderKind::DOCKER,
            recorded_runtime("/workspace", None, None, None),
        );
        let spec = replacement_sandbox_spec(
            RunId::new(),
            &replacement_run_spec(),
            &recorded,
            fabro_sandbox::driver::ProviderAccess::default(),
            None,
        )
        .expect("the empty-workspace replacement spec builds");

        assert!(spec.clone.skip);
        assert_eq!(spec.clone.origin_url, None);
    }

    /// A non-GitHub origin is refused by the clone planner before any
    /// provider is contacted — the failure surfaces without a Docker
    /// daemon ever being needed.
    #[tokio::test]
    async fn replacement_build_refuses_a_non_github_origin_with_a_clear_error() {
        let recorded = recorded_instance(
            fabro_types::SandboxProviderKind::DOCKER,
            recorded_runtime(
                "/workspace/widget",
                Some(true),
                Some("https://gitlab.com/acme/widget"),
                Some("main"),
            ),
        );
        let spec = replacement_sandbox_spec(
            RunId::new(),
            &replacement_run_spec(),
            &recorded,
            fabro_sandbox::driver::ProviderAccess::default(),
            None,
        )
        .expect("the spec itself builds");

        let Err(error) = spec.build(None).await else {
            panic!("a GitLab origin cannot be cloned")
        };
        let chain = format!("{error:#}");
        assert!(
            chain.contains("GitHub repository origins only"),
            "the error names the clone-source contract: {chain}"
        );
    }

    /// A terminal run whose recorded sandbox is gone from the provider gets
    /// a freshly provisioned one for the turn (fabro-8d30 part b). The
    /// local host sandbox is identified by its working directory, so
    /// removing the directory is exactly what "the sandbox no longer
    /// exists" looks like: reconnect fails, provisioning re-designates the
    /// recorded directory, and the turn answers instead of failing with
    /// sandbox_unavailable.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_missing_terminal_run_sandbox_is_provisioned_fresh_for_the_turn() {
        let twin = twin_openai().await;
        let namespace = format!("{}::{}", module_path!(), line!());
        TwinScenarios::new(namespace.clone())
            .scenario(
                TwinScenario::responses(MODEL)
                    .input_contains("Question for a reaped run")
                    .text("Answer from a fresh sandbox"),
            )
            .load(twin)
            .await;
        let state = twin_backed_state(twin.base_url.clone(), &namespace);
        spawn_scheduler(Arc::clone(&state));
        let app = build_test_router(Arc::clone(&state));
        let workspace = tempfile::tempdir().unwrap();
        let run_id = completed_run(&app, workspace.path()).await;

        let run_store = state.store_ref().open_run_reader(&run_id).await.unwrap();
        let projection = run_store.state().await.unwrap();
        let recorded = projection
            .sandbox
            .as_ref()
            .and_then(fabro_types::RunSandbox::instance)
            .expect("the completed run recorded its sandbox")
            .clone();
        assert_eq!(
            recorded.provider,
            fabro_types::SandboxProviderKind::LOCAL,
            "the dry run executes on the local provider"
        );
        let working_directory = std::path::PathBuf::from(&recorded.runtime.working_directory);
        std::fs::remove_dir_all(&working_directory).expect("the recorded workspace is removable");

        let created = json_response(
            &app,
            post_json(
                &format!("/runs/{run_id}/sessions"),
                &serde_json::json!({ "title": "Ask Fabro", "model": MODEL }),
            ),
            StatusCode::CREATED,
        )
        .await;
        let session_id: SessionId = created["id"].as_str().unwrap().parse().unwrap();

        turn(&app, session_id, "Question for a reaped run").await;

        assert!(
            working_directory.is_dir(),
            "fresh provisioning re-designated the recorded working directory"
        );
    }

    /// When provisioning itself fails, the build error is the retryable
    /// sandbox_unavailable one and names both the provisioning attempt and
    /// the reconnect failure that triggered it — never a panic and never a
    /// silent fallback.
    #[tokio::test]
    async fn a_provisioning_failure_surfaces_as_retryable_sandbox_unavailable() {
        let state = TestAppStateBuilder::new().build();
        let mut graph = fabro_types::Graph::new("test");
        for node_id in ["start", "exit"] {
            graph
                .nodes
                .insert(node_id.to_string(), fabro_types::Node::new(node_id));
        }
        let spec = fabro_types::RunSpec {
            run_id: RunId::new(),
            settings: fabro_types::WorkflowSettings::default(),
            graph,
            graph_source: None,
            workflow_slug: None,
            workflow_version_id: None,
            target: None,
            automation: None,
            source_directory: None,
            labels: std::collections::HashMap::default(),
            provenance: fabro_types::test_support::test_run_provenance(),
            definition_blob: None,
            spec_blob: None,
            git: None,
            fork_source_ref: None,
        };
        // A provider kind the server never configured: connect refuses it
        // before anything external is contacted.
        let recorded = fabro_types::RunSandboxInstance {
            provider: fabro_types::SandboxProviderKind::try_new("ghost")
                .expect("ghost is a valid kind"),
            image:    None,
            snapshot: None,
            runtime:  fabro_types::RunSandboxRuntime {
                id:                "gone".to_string(),
                working_directory: "/workspace".to_string(),
                repo_cloned:       None,
                clone_origin_url:  None,
                clone_branch:      None,
                workspace_root:    None,
                repos_root:        None,
                primary_repo_path: None,
                primary_repo_link: None,
            },
        };

        let Err(error) = provision_replacement_sandbox(
            &state,
            spec.run_id,
            &spec,
            &recorded,
            &fabro_sandbox::driver::ProviderAccess::default(),
            anyhow::anyhow!("the recorded sandbox is gone"),
        )
        .await
        else {
            panic!("an unconfigured provider cannot provision")
        };

        assert!(
            matches!(error, sessions::AskFabroBuildError::SandboxUnavailable(_)),
            "got {error:?}"
        );
        assert!(error.retryable());
        let text = error.to_string();
        assert!(
            text.contains("provisioning a fresh ghost sandbox"),
            "the error names the provisioning attempt: {text}"
        );
        assert!(
            text.contains("the recorded sandbox is gone"),
            "the error carries the reconnect failure: {text}"
        );
        assert!(
            text.contains("not configured"),
            "the error carries the provisioning cause: {text}"
        );
    }
}
