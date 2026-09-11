use async_trait::async_trait;
use fabro_tool::{
    PackagedWorkflowVersions, ToolError, ValidatedWorkflowVersionCreate, WorkflowVersionPackager,
};
use tokio::task;
use tracing::warn;

/// Packages supplied workflow contents for standalone MCP and capable run
/// workers; the backend that owns the API client performs registration.
pub struct ServerWorkflowVersionPackager;

const PACKAGING_FAILED: &str = "workflow source could not be packaged; check configuration, \
                                syntax, local references, and package limits";

#[async_trait]
impl WorkflowVersionPackager for ServerWorkflowVersionPackager {
    async fn package(
        &self,
        source: ValidatedWorkflowVersionCreate,
    ) -> anyhow::Result<PackagedWorkflowVersions> {
        let closure = task::spawn_blocking(move || {
            fabro_manifest::collect_supplied_workflow_versions(&source.entrypoint, &source.files)
        })
        .await
        .map_err(|err| anyhow::anyhow!("workflow packaging task failed: {err}"))?
        .map_err(|err| {
            // Parser diagnostics may quote supplied source, so the chain stays
            // in the log and only a generic message crosses the tool boundary.
            warn!(error = %format!("{err:#}"), "workflow version packaging failed");
            ToolError::message(PACKAGING_FAILED)
        })?;
        Ok(PackagedWorkflowVersions {
            root_id:  closure.root_id(),
            versions: closure
                .versions()
                .map(|(_, version)| version.version().clone())
                .collect(),
        })
    }
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

    #[tokio::test]
    async fn packager_returns_dependencies_before_root() {
        let packaged = ServerWorkflowVersionPackager
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
        let mut oversized = fixture();
        oversized.files.insert(
            "prompt.md".parse().unwrap(),
            "\u{1}".repeat(fabro_types::MAX_WORKFLOW_VERSION_FILE_BYTES - 1),
        );
        for input in [
            invalid_root,
            oversized,
            source("workflow", &[(
                "workflow",
                "PRIVATE_CONTENT invalid source",
            )]),
        ] {
            let error = ServerWorkflowVersionPackager
                .package(input)
                .await
                .unwrap_err();
            let rendered = format!("{error:#}");
            assert!(!rendered.contains("PRIVATE_CONTENT"), "{rendered}");
            assert_eq!(rendered, PACKAGING_FAILED);
        }
        let mut wrong_case = fixture();
        let prompt = wrong_case
            .files
            .remove(&"prompt.md".parse().unwrap())
            .unwrap();
        wrong_case
            .files
            .insert("Prompt.md".parse().unwrap(), prompt);
        assert!(
            ServerWorkflowVersionPackager
                .package(wrong_case)
                .await
                .is_err(),
            "a case-insensitive host must not satisfy an exact reference"
        );
    }
}
