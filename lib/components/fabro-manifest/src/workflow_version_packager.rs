//! Application adapter that packages caller-supplied workflow contents for
//! the `fabro_workflow_version_create` tool.

use async_trait::async_trait;
use fabro_tool::{
    PackagedWorkflowVersions, ToolError, ValidatedWorkflowVersionCreate, WorkflowVersionPackager,
};
use fabro_workflow_version::WorkflowVersionError;
use tokio::task;
use tracing::warn;

use crate::WorkflowVersionCollectError;

/// Packages supplied workflow contents for standalone MCP and capable run
/// workers; the backend that owns the API client performs registration.
pub struct SuppliedWorkflowVersionPackager;

const PACKAGING_HINT: &str = "check configuration, syntax, local references, and package limits";

#[async_trait]
impl WorkflowVersionPackager for SuppliedWorkflowVersionPackager {
    async fn package(
        &self,
        source: ValidatedWorkflowVersionCreate,
    ) -> anyhow::Result<PackagedWorkflowVersions> {
        task::spawn_blocking(move || {
            let closure =
                crate::collect_supplied_workflow_versions(&source.entrypoint, &source.files)
                    .map_err(|err| {
                        warn!(error = %format!("{err:#}"), "workflow version packaging failed");
                        ToolError::message(render_packaging_error(&err))
                    })?;
            Ok(PackagedWorkflowVersions {
                root_id:  closure.root_id(),
                versions: closure.into_versions(),
            })
        })
        .await
        .map_err(|err| anyhow::anyhow!("workflow packaging task failed: {err}"))?
    }
}

/// Render a packaging failure for the tool caller. Every collector variant's
/// own message names paths and counts only, so most render their full cause
/// chain and the caller can fix the input. The graph parser, TOML parser, and
/// template engine quote the offending source in their diagnostics, so
/// failures that reach them stop at the last path-only level and add a hint.
fn render_packaging_error(err: &WorkflowVersionCollectError) -> String {
    let quotes_source = match err {
        WorkflowVersionCollectError::Collect { .. } => true,
        WorkflowVersionCollectError::InvalidVersion { source, .. } => matches!(
            source,
            WorkflowVersionError::GraphParse { .. }
                | WorkflowVersionError::Template { .. }
                | WorkflowVersionError::Config { .. }
        ),
        _ => false,
    };
    if !quotes_source {
        return fabro_util::error::collect_chain(err).join(": ");
    }
    let summary = match err {
        // `WorkflowVersionError` names the offending path; only its source
        // quotes content.
        WorkflowVersionCollectError::InvalidVersion { source, .. } => format!("{err}: {source}"),
        _ => err.to_string(),
    };
    format!("{summary}; {PACKAGING_HINT}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(entrypoint: &str, files: &[(&str, &str)]) -> ValidatedWorkflowVersionCreate {
        ValidatedWorkflowVersionCreate {
            entrypoint: entrypoint.parse().unwrap(),
            files:      files
                .iter()
                .map(|(path, content)| (path.parse().unwrap(), (*content).to_string()))
                .collect(),
        }
    }

    fn fixture() -> ValidatedWorkflowVersionCreate {
        source("workflow.toml", &[
            (
                "workflow.toml",
                "_version = 1\n[workflow]\ngraph = \"workflow.fabro\"\n",
            ),
            (
                "workflow.fabro",
                r#"digraph W { p [prompt="@prompt.md"] child [stack.child_workflow="child.fabro"] }"#,
            ),
            ("prompt.md", "Review the implementation."),
            ("child.fabro", "digraph Child {}"),
        ])
    }

    async fn package_error(input: ValidatedWorkflowVersionCreate) -> String {
        let error = SuppliedWorkflowVersionPackager
            .package(input)
            .await
            .unwrap_err();
        format!("{error:#}")
    }

    #[tokio::test]
    async fn packager_returns_dependencies_before_root() {
        let packaged = SuppliedWorkflowVersionPackager
            .package(fixture())
            .await
            .unwrap();
        assert_eq!(packaged.versions.len(), 2);
        assert_eq!(packaged.versions[0].entrypoint().as_str(), "child.fabro");
        assert_eq!(
            packaged.versions[1].id().unwrap(),
            packaged.root_id,
            "root version must be last"
        );
        let child_id = packaged.versions[0].id().unwrap();
        assert!(
            packaged.versions[1]
                .workflow_dependencies()
                .values()
                .any(|id| *id == child_id)
        );
    }

    #[tokio::test]
    async fn packaging_errors_never_quote_supplied_source() {
        let mut invalid_root = fixture();
        // Child is valid, but the root fails after its dependency is assembled.
        invalid_root.files.insert(
            "workflow.toml".parse().unwrap(),
            "_version = 1\n[workflow]\ngraph = \"workflow.fabro\"\n[run.goal]\nfile = \"missing.md\""
                .into(),
        );
        let mut invalid_config = fixture();
        invalid_config.files.insert(
            "workflow.toml".parse().unwrap(),
            "_version = 1\nPRIVATE_CONTENT = [unterminated".into(),
        );
        for input in [
            invalid_root,
            invalid_config,
            source("workflow", &[(
                "workflow",
                "PRIVATE_CONTENT invalid source",
            )]),
        ] {
            let rendered = package_error(input).await;
            assert!(!rendered.contains("PRIVATE_CONTENT"), "{rendered}");
        }
    }

    #[tokio::test]
    async fn path_only_failures_tell_the_caller_what_to_fix() {
        let mut missing_child = fixture();
        missing_child.files.remove(&"child.fabro".parse().unwrap());
        let rendered = package_error(missing_child).await;
        assert!(
            rendered.contains("`child.fabro`") && rendered.contains("missing"),
            "{rendered}"
        );

        let mut wrong_case = fixture();
        let prompt = wrong_case
            .files
            .remove(&"prompt.md".parse().unwrap())
            .unwrap();
        wrong_case
            .files
            .insert("Prompt.md".parse().unwrap(), prompt);
        // A case-insensitive host reads the file and reports the unsupplied
        // key; a case-sensitive host reports it missing. Either names the
        // path the graph asked for.
        let rendered = package_error(wrong_case).await;
        assert!(rendered.contains("`prompt.md`"), "{rendered}");

        let mut oversized = fixture();
        oversized.files.insert(
            "prompt.md".parse().unwrap(),
            "\u{1}".repeat(fabro_types::MAX_WORKFLOW_VERSION_FILE_BYTES - 1),
        );
        let rendered = package_error(oversized).await;
        assert!(rendered.contains("canonical bytes"), "{rendered}");

        let rendered = package_error(source("workflow", &[(
            "workflow",
            "PRIVATE_CONTENT invalid source",
        )]))
        .await;
        assert!(rendered.ends_with(PACKAGING_HINT), "{rendered}");
    }
}
