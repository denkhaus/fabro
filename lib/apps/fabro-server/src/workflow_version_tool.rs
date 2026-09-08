use std::path::Path;

use anyhow::Context as _;
use async_trait::async_trait;
use fabro_client::Client;
use fabro_config::project::WorkflowLocation;
use fabro_manifest::CollectedWorkflowClosure;
use fabro_tool::{FabroWorkflowVersionCreateParams, ToolError, WorkflowVersionCreateAdapter};
use fabro_types::WorkflowVersionId;
use tokio::task;

/// Content-only registration shared by standalone MCP and capable run workers.
pub struct ServerWorkflowVersionCreateAdapter;

#[async_trait]
impl WorkflowVersionCreateAdapter for ServerWorkflowVersionCreateAdapter {
    async fn create_workflow_version(
        &self,
        params: FabroWorkflowVersionCreateParams,
        client: &Client,
    ) -> anyhow::Result<WorkflowVersionId> {
        params.validate()?;
        let closure = task::spawn_blocking(move || {
            let staging = tempfile::Builder::new().prefix("fabro-workflow-version-").tempdir()?;
            collect_supplied_workflow(&params, staging)
        })
            .await
            .context("workflow packaging task failed")?
            // Parser diagnostics may contain supplied source. Keep them off the
            // tool result boundary, including nested error chains.
            .map_err(|_| ToolError::message("workflow source could not be packaged; check configuration, syntax, local references, and package limits"))?;
        let versions = closure
            .versions()
            .map(|(_, version)| version.version())
            .collect::<Vec<_>>();
        client.register_workflow_versions(versions).await?;
        Ok(closure.root_id())
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "bounded file staging and collection run on spawn_blocking"
)]
fn collect_supplied_workflow(
    params: &FabroWorkflowVersionCreateParams,
    staging: tempfile::TempDir,
) -> anyhow::Result<CollectedWorkflowClosure> {
    // TempDir owns cleanup on every return path, including collection errors.
    let root = staging.path().canonicalize()?;
    for (path, contents) in &params.files {
        let destination = root.join(path.as_str());
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(destination, contents)?;
    }
    let location = WorkflowLocation::from_exact_path(Path::new(params.entrypoint.as_str()), &root)?;
    let closure = fabro_manifest::collect_workflow_versions_at_location(
        &location,
        &root,
        Path::new(params.entrypoint.as_str()),
    )?;
    // Collection validates the whole closure, including serialized request budgets.
    // A case-insensitive host must not satisfy a missing exact source key.
    for (_, version) in closure.versions() {
        for (path, content) in version.version().files() {
            anyhow::ensure!(
                params.files.get(path) == Some(content),
                "collected file does not match supplied source"
            );
        }
    }
    drop(staging);
    Ok(closure)
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::disallowed_methods,
        reason = "hermetic temporary source fixtures"
    )]
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    use axum::http::StatusCode;
    use axum::routing::post;
    use axum::{Json, Router};
    use fabro_types::WorkflowVersion;
    use serde_json::json;

    use super::*;

    fn params(entrypoint: &str, files: &[(&str, &str)]) -> FabroWorkflowVersionCreateParams {
        FabroWorkflowVersionCreateParams {
            entrypoint: entrypoint.parse().unwrap(),
            files:      files
                .iter()
                .map(|(path, content)| (path.parse().unwrap(), (*content).to_string()))
                .collect(),
        }
    }

    fn fixture() -> FabroWorkflowVersionCreateParams {
        params("workflow.toml", &[
            (
                "workflow.toml",
                "_version = 1\n[workflow]\ngraph = \"workflow.fabro\"\n",
            ),
            (
                "workflow.fabro",
                r#"digraph W { p [prompt="@prompt.md"] child [stack.child_workflow="child.fabro"] }"#,
            ),
            (
                "prompt.md",
                "Keep {{ secrets.TEST }} and {{ env.TEST }} for runtime.",
            ),
            ("child.fabro", "digraph Child {}"),
        ])
    }

    fn collect(params: &FabroWorkflowVersionCreateParams) -> CollectedWorkflowClosure {
        params.validate().unwrap();
        collect_supplied_workflow(params, tempfile::tempdir().unwrap()).unwrap()
    }

    #[test]
    fn workflow_version_content_matches_existing_collector_and_cleans_staging() {
        for input in [
            params("workflow.fabro", &[("workflow.fabro", "digraph W {}")]),
            fixture(),
        ] {
            let source = tempfile::tempdir().unwrap();
            for (path, content) in &input.files {
                std::fs::write(source.path().join(path.as_str()), content).unwrap();
            }
            let expected = fabro_manifest::collect_workflow_versions(
                Path::new(input.entrypoint.as_str()),
                source.path(),
            )
            .unwrap();
            let staging = tempfile::tempdir().unwrap();
            let path = staging.path().to_owned();
            let actual = collect_supplied_workflow(&input, staging).unwrap();
            assert!(!path.exists());
            assert_eq!(actual.root_id(), expected.root_id());
            assert_eq!(
                actual
                    .versions()
                    .map(|(_, v)| v.version())
                    .collect::<Vec<_>>(),
                expected
                    .versions()
                    .map(|(_, v)| v.version())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn workflow_version_exact_extensionless_entrypoint_and_child_ignore_selectors() {
        let input = params("workflow", &[
            (
                "workflow",
                r#"digraph W { child [stack.child_workflow="child"] }"#,
            ),
            ("child", "digraph Child {}"),
            (
                ".fabro/project.toml",
                "malformed project config must not be read",
            ),
            (
                ".fabro/workflows/workflow/workflow.toml",
                "misleading named workflow",
            ),
        ]);
        let closure = collect(&input);
        let versions = closure.versions().collect::<Vec<_>>();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[1].1.version().entrypoint().as_str(), "workflow");
        assert_eq!(versions[0].1.version().entrypoint().as_str(), "child");
    }

    #[test]
    fn workflow_version_rejects_missing_and_escaping_references_and_cleans_failure() {
        let parent = tempfile::tempdir().unwrap();
        std::fs::write(
            parent.path().join("outside.md"),
            "host content must never satisfy a reference",
        )
        .unwrap();
        std::fs::write(parent.path().join("child.fabro"), "digraph Host {}").unwrap();
        for (index, input) in [
            params("workflow.fabro", &[
                ("workflow.fabro", r#"digraph W { p [prompt="@prompt.md"] }"#),
                ("Prompt.md", "wrong case"),
            ]),
            params("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [prompt="@../outside.md"] }"#,
            )]),
            params("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [prompt="@sub/../../outside.md"] }"#,
            )]),
            params("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [prompt="@outside.md"] }"#,
            )]),
            params("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [output_schema="@../outside.md"] }"#,
            )]),
            params("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [stack.child_workflow="../child.fabro"] }"#,
            )]),
            params("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [stack.child_workflow="sub/../../child.fabro"] }"#,
            )]),
            params("workflow.fabro", &[(
                "workflow.fabro",
                r#"digraph W { p [stack.child_workflow="missing"] }"#,
            )]),
            params("workflow.toml", &[(
                "workflow.toml",
                "_version = 1\n[workflow]\ngraph = \"../child.fabro\"\n",
            )]),
            params("workflow.fabro", &[(
                "workflow.fabro",
                "invalid source PRIVATE_CONTENT",
            )]),
        ]
        .into_iter()
        .enumerate()
        {
            let staging = tempfile::tempdir_in(parent.path()).unwrap();
            let path = staging.path().to_owned();
            assert!(
                collect_supplied_workflow(&input, staging).is_err(),
                "accepted invalid fixture {index}"
            );
            assert!(!path.exists());
        }
    }

    #[test]
    fn workflow_version_registration_preserves_literal_scripts_without_executing() {
        let directory = tempfile::tempdir().unwrap();
        let marker = directory.path().join("must-not-exist");
        // Script is literal command text in Fabro, not an @file import.
        let graph = format!(
            "digraph W {{ command [script=\"touch {}\"] }}",
            marker.display()
        );
        let input = params("workflow", &[("workflow", &graph)]);
        let closure = collect(&input);
        let root = closure.versions().last().unwrap().1.version();
        assert_eq!(root.files()[&"workflow".parse().unwrap()], graph);
        assert!(!marker.exists());
    }

    #[test]
    fn workflow_version_map_order_is_irrelevant_and_reachable_changes_change_ids() {
        let input = fixture();
        let first = collect(&input);
        let mut reordered = input.clone();
        reordered.files = input.files.into_iter().rev().collect();
        assert_eq!(first.root_id(), collect(&reordered).root_id());
        reordered
            .files
            .insert("prompt.md".parse().unwrap(), "changed".into());
        let changed = collect(&reordered);
        assert_ne!(first.root_id(), changed.root_id());
        assert_eq!(
            first.versions().next().unwrap().0,
            changed.versions().next().unwrap().0
        );
        reordered
            .files
            .insert("child.fabro".parse().unwrap(), "digraph Changed {}".into());
        let changed_child = collect(&reordered);
        assert_ne!(changed.root_id(), changed_child.root_id());
        assert_ne!(
            changed.versions().next().unwrap().0,
            changed_child.versions().next().unwrap().0
        );
    }

    #[tokio::test]
    async fn workflow_version_uploads_dependencies_first_and_retries_immutable_content() {
        let uploads = Arc::new(Mutex::new(Vec::<WorkflowVersion>::new()));
        let seen = uploads.clone();
        let app = Router::new().route(
            "/api/v1/workflow-versions",
            post(move |Json(version): Json<WorkflowVersion>| {
                let seen = seen.clone();
                async move {
                    let mut seen = seen.lock().unwrap();
                    for id in version.workflow_dependencies().values() {
                        assert!(
                            seen.iter().any(|prior| prior.id().unwrap() == *id),
                            "dependency must be registered first"
                        );
                    }
                    let id = version.id().unwrap();
                    seen.push(version);
                    (StatusCode::CREATED, Json(json!({"workflow_version_id":id})))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client =
            Client::new_no_proxy(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let expected = collect(&fixture()).root_id();
        for _ in 0..2 {
            let id = ServerWorkflowVersionCreateAdapter
                .create_workflow_version(fixture(), &client)
                .await
                .unwrap();
            assert_eq!(id, expected);
        }
        assert_eq!(uploads.lock().unwrap().len(), 4);
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn workflow_version_invalid_closure_has_no_uploads_or_source_in_errors() {
        let server = httpmock::MockServer::start_async().await;
        let upload = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST);
                then.status(500);
            })
            .await;
        let client = Client::new_no_proxy(&server.url("")).unwrap();
        let mut invalid = fixture();
        // Child is valid, but the root fails after its dependency is assembled.
        invalid.files.insert("workflow.toml".parse().unwrap(), "_version = 1\n[workflow]\ngraph = \"workflow.fabro\"\n[run.goal]\nfile = \"missing.md\"".into());
        let mut oversized = fixture();
        let huge_prompt = "\u{1}".repeat(fabro_types::MAX_WORKFLOW_VERSION_FILE_BYTES - 1);
        oversized
            .files
            .insert("prompt.md".parse().unwrap(), huge_prompt);
        for input in [
            invalid,
            oversized,
            params("workflow", &[(
                "workflow",
                "PRIVATE_CONTENT invalid source",
            )]),
        ] {
            let error = ServerWorkflowVersionCreateAdapter
                .create_workflow_version(input, &client)
                .await
                .unwrap_err();
            assert!(!format!("{error:#}").contains("PRIVATE_CONTENT"));
        }
        upload.assert_calls_async(0).await;
    }

    #[tokio::test]
    async fn workflow_version_root_upload_failure_leaves_child_for_safe_retry() {
        let server = httpmock::MockServer::start_async().await;
        let closure = collect(&fixture());
        let child = closure.versions().next().unwrap().1.version();
        let root = closure.versions().last().unwrap().1.version();
        let child_upload = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST)
                    .path("/api/v1/workflow-versions")
                    .json_body_obj(child);
                then.status(201)
                    .json_body(json!({"workflow_version_id":child.id().unwrap()}));
            })
            .await;
        let failed_root = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST)
                    .path("/api/v1/workflow-versions")
                    .json_body_obj(root);
                then.status(400);
            })
            .await;
        let deletion = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::DELETE);
                then.status(500);
            })
            .await;
        let client = Client::new_no_proxy(&server.url("")).unwrap();
        assert!(
            ServerWorkflowVersionCreateAdapter
                .create_workflow_version(fixture(), &client)
                .await
                .is_err()
        );
        child_upload.assert_calls_async(1).await;
        failed_root.assert_calls_async(1).await;
        failed_root.delete_async().await;
        let root_upload = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST)
                    .path("/api/v1/workflow-versions")
                    .json_body_obj(root);
                then.status(201)
                    .json_body(json!({"workflow_version_id":root.id().unwrap()}));
            })
            .await;
        assert_eq!(
            ServerWorkflowVersionCreateAdapter
                .create_workflow_version(fixture(), &client)
                .await
                .unwrap(),
            closure.root_id()
        );
        child_upload.assert_calls_async(2).await;
        root_upload.assert_calls_async(1).await;
        deletion.assert_calls_async(0).await;
    }

    #[tokio::test]
    async fn workflow_version_failed_upload_and_wrong_server_id_never_return_success() {
        for wrong_id in [false, true] {
            let server = httpmock::MockServer::start_async().await;
            let closure = collect(&fixture());
            let child = closure.versions().next().unwrap().1.version();
            let upload = server.mock_async(|when, then| {
                when.method(httpmock::Method::POST).path("/api/v1/workflow-versions").json_body_obj(child);
                if wrong_id {
                    then.status(201).json_body(json!({"workflow_version_id": WorkflowVersionId::from(fabro_types::BlobHash::new(b"wrong"))}));
                } else { then.status(400); }
            }).await;
            let root = closure.versions().last().unwrap().1.version();
            let root_upload = server
                .mock_async(|when, then| {
                    when.method(httpmock::Method::POST)
                        .path("/api/v1/workflow-versions")
                        .json_body_obj(root);
                    then.status(201)
                        .json_body(json!({"workflow_version_id":root.id().unwrap()}));
                })
                .await;
            let client = Client::new_no_proxy(&server.url("")).unwrap();
            assert!(
                ServerWorkflowVersionCreateAdapter
                    .create_workflow_version(fixture(), &client)
                    .await
                    .is_err()
            );
            upload.assert_calls_async(1).await;
            root_upload.assert_calls_async(0).await;
        }
    }
}
