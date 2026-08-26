use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use fabro_api::types::RunManifest;
use fabro_automation::AutomationId;
use fabro_config::{EnvironmentLayer, MergeMap};
use fabro_manifest::ManifestBuildInput;
use fabro_types::{GitHubRepositorySlug, GitRunTarget, RunId, RunTarget, TargetValidationError};
use tokio::{fs, task};

use crate::git_checkout::{
    GitCheckoutError, GitRepoCache, WorktreePrepareInput, resolve_git_auth_config,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AutomationRunMaterializeInput {
    pub automation_id:      AutomationId,
    pub target:             GitRunTarget,
    pub workflow:           String,
    pub run_id:             RunId,
    pub user_settings_path: PathBuf,
    pub temp_root:          PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct AutomationRunMaterialized {
    pub manifest:                 RunManifest,
    pub submitted_manifest_bytes: Vec<u8>,
    pub target:                   GitRunTarget,
}

#[derive(thiserror::Error, Debug)]
pub(crate) enum RunMaterializeError {
    #[error("invalid automation Git target")]
    InvalidTarget {
        #[source]
        source: TargetValidationError,
    },
    #[error("failed to prepare automation checkout")]
    Checkout {
        #[from]
        source: GitCheckoutError,
    },
    #[error("failed to prepare automation temporary directory {path}")]
    TempDirectory {
        path:   PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to resolve automation workflow")]
    WorkflowNotFound {
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to build run manifest")]
    Manifest {
        #[source]
        source: anyhow::Error,
    },
    #[error("manifest build task failed")]
    ManifestTask {
        #[source]
        source: task::JoinError,
    },
    #[error("failed to serialize materialized run manifest")]
    SerializeManifest {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to load GitHub credentials")]
    Credentials {
        #[source]
        source: anyhow::Error,
    },
}

#[async_trait]
pub(crate) trait AutomationRunMaterializer: Send + Sync {
    async fn materialize(
        &self,
        input: AutomationRunMaterializeInput,
    ) -> Result<AutomationRunMaterialized, RunMaterializeError>;
}

#[derive(Clone)]
pub(crate) struct ProductionAutomationRunMaterializer {
    github_credentials:   Option<fabro_github::GitHubCredentials>,
    github_api_base_url:  String,
    http_client:          Option<fabro_http::HttpClient>,
    environment_defaults: MergeMap<EnvironmentLayer>,
    repo_cache:           Arc<GitRepoCache>,
}

impl ProductionAutomationRunMaterializer {
    pub(crate) fn new(
        github_credentials: Option<fabro_github::GitHubCredentials>,
        github_api_base_url: String,
        http_client: Option<fabro_http::HttpClient>,
        environment_defaults: MergeMap<EnvironmentLayer>,
        repo_cache: Arc<GitRepoCache>,
    ) -> Self {
        Self {
            github_credentials,
            github_api_base_url,
            http_client,
            environment_defaults,
            repo_cache,
        }
    }
}

#[async_trait]
impl AutomationRunMaterializer for ProductionAutomationRunMaterializer {
    async fn materialize(
        &self,
        input: AutomationRunMaterializeInput,
    ) -> Result<AutomationRunMaterialized, RunMaterializeError> {
        let repo = GitHubRepositorySlug::try_new(&input.target.repo).ok_or(
            RunMaterializeError::InvalidTarget {
                source: TargetValidationError::Repository,
            },
        )?;
        fs::create_dir_all(&input.temp_root)
            .await
            .map_err(|source| RunMaterializeError::TempDirectory {
                path: input.temp_root.clone(),
                source,
            })?;
        let temp_dir = tempfile::Builder::new()
            .prefix(&format!(
                "automation-{}-{}-",
                input.automation_id.as_str(),
                input.run_id
            ))
            .tempdir_in(&input.temp_root)
            .map_err(|source| RunMaterializeError::TempDirectory {
                path: input.temp_root.clone(),
                source,
            })?;
        let checkout_dir = temp_dir.path().join("repo");
        let auth = resolve_git_auth_config(
            self.github_credentials.as_ref(),
            &repo,
            &self.github_api_base_url,
            self.http_client.clone(),
        )
        .await
        .map_err(|source| RunMaterializeError::Credentials { source })?;

        let checked_out_sha = self
            .repo_cache
            .prepare_worktree(WorktreePrepareInput {
                repo:         &repo,
                target:       &input.target,
                auth:         auth.as_ref(),
                worktree_dir: &checkout_dir,
            })
            .await?;

        let mut exact_target = input.target;
        exact_target.sha = Some(checked_out_sha);

        let manifest_input = ManifestFromCheckoutInput {
            workflow: input.workflow,
            user_settings_path: input.user_settings_path,
            checkout_dir,
            target: exact_target,
            environment_defaults: self.environment_defaults.clone(),
        };
        task::spawn_blocking(move || build_manifest_from_checkout(manifest_input))
            .await
            .map_err(|source| RunMaterializeError::ManifestTask { source })?
    }
}

#[derive(Debug)]
pub(crate) struct ManifestFromCheckoutInput {
    workflow:             String,
    user_settings_path:   PathBuf,
    checkout_dir:         PathBuf,
    target:               GitRunTarget,
    environment_defaults: MergeMap<EnvironmentLayer>,
}

fn build_manifest_from_checkout(
    args: ManifestFromCheckoutInput,
) -> Result<AutomationRunMaterialized, RunMaterializeError> {
    let ManifestFromCheckoutInput {
        workflow,
        user_settings_path,
        checkout_dir,
        target,
        environment_defaults,
    } = args;
    // Re-validating the exact target (now carrying the checked-out SHA) yields
    // the same `GitContext` projection the run-intent path uses.
    let validated = RunTarget::Git(target)
        .validate()
        .map_err(|source| RunMaterializeError::InvalidTarget { source })?;
    let RunTarget::Git(target) = validated.target else {
        unreachable!("validating a Git target yields a Git target");
    };
    let built = fabro_manifest::build_run_manifest(ManifestBuildInput {
        workflow: workflow.into(),
        cwd: checkout_dir,
        user_settings_path: Some(user_settings_path),
        environment_defaults,
        ..ManifestBuildInput::default()
    })
    .map_err(manifest_build_error)?;

    let mut manifest = built.manifest;
    manifest.git = validated.git;
    let submitted_manifest_bytes = serde_json::to_vec(&manifest)
        .map_err(|source| RunMaterializeError::SerializeManifest { source })?;
    Ok(AutomationRunMaterialized {
        manifest,
        submitted_manifest_bytes,
        target,
    })
}

fn manifest_build_error(error: anyhow::Error) -> RunMaterializeError {
    if error.chain().any(|source| {
        source
            .downcast_ref::<fabro_config::Error>()
            .is_some_and(|err| matches!(err, fabro_config::Error::WorkflowNotFound(_)))
    }) {
        RunMaterializeError::WorkflowNotFound { source: error }
    } else {
        RunMaterializeError::Manifest { source: error }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
pub struct TestAutomationRunMaterializer {
    inner: std::sync::Arc<std::sync::Mutex<TestAutomationRunMaterializerState>>,
}

#[cfg(any(test, feature = "test-support"))]
struct TestAutomationRunMaterializerState {
    captured_inputs: Vec<AutomationRunMaterializeInput>,
    response:        Result<Box<AutomationRunMaterialized>, TargetValidationError>,
}

#[cfg(any(test, feature = "test-support"))]
impl TestAutomationRunMaterializer {
    pub fn succeed(
        manifest: RunManifest,
        submitted_manifest_bytes: Vec<u8>,
        target: GitRunTarget,
    ) -> Self {
        Self::new(Ok(Box::new(AutomationRunMaterialized {
            manifest,
            submitted_manifest_bytes,
            target,
        })))
    }

    pub fn fail_invalid_target() -> Self {
        Self::new(Err(TargetValidationError::Repository))
    }

    fn new(response: Result<Box<AutomationRunMaterialized>, TargetValidationError>) -> Self {
        Self {
            inner: std::sync::Arc::new(std::sync::Mutex::new(TestAutomationRunMaterializerState {
                captured_inputs: Vec::new(),
                response,
            })),
        }
    }

    pub(crate) fn captured_inputs(&self) -> Vec<AutomationRunMaterializeInput> {
        self.inner
            .lock()
            .expect("test automation materializer lock poisoned")
            .captured_inputs
            .clone()
    }

    pub(crate) fn into_materializer(self) -> std::sync::Arc<dyn AutomationRunMaterializer> {
        std::sync::Arc::new(self)
    }
}

#[cfg(any(test, feature = "test-support"))]
#[async_trait]
impl AutomationRunMaterializer for TestAutomationRunMaterializer {
    async fn materialize(
        &self,
        input: AutomationRunMaterializeInput,
    ) -> Result<AutomationRunMaterialized, RunMaterializeError> {
        let mut guard = self
            .inner
            .lock()
            .expect("test automation materializer lock poisoned");
        guard.captured_inputs.push(input);
        guard
            .response
            .clone()
            .map(|materialized| *materialized)
            .map_err(|source| RunMaterializeError::InvalidTarget { source })
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::disallowed_methods,
        reason = "Materializer unit tests write small temporary workflow fixtures synchronously."
    )]

    use std::collections::HashMap;
    use std::fs;

    use fabro_types::DirtyStatus;
    use tempfile::TempDir;

    use super::*;

    fn test_environment_defaults() -> MergeMap<EnvironmentLayer> {
        MergeMap::from(HashMap::from([("default".to_string(), EnvironmentLayer {
            provider: Some("local".to_string()),
            ..EnvironmentLayer::default()
        })]))
    }

    #[test]
    fn manifest_builder_uses_checkout_for_workflow_and_separate_git_context() {
        let temp = TempDir::new().unwrap();
        let checkout = temp.path().join("checkout");
        let workflow_dir = checkout.join(".fabro/workflows/demo");
        fs::create_dir_all(&workflow_dir).unwrap();
        fs::write(checkout.join(".fabro/project.toml"), "_version = 1\n").unwrap();
        fs::write(
            workflow_dir.join("workflow.fabro"),
            r#"digraph Demo { graph [goal="Ship automation"] start [shape=Mdiamond] exit [shape=Msquare] start -> exit }"#,
        )
        .unwrap();
        fs::write(
            workflow_dir.join("workflow.toml"),
            "_version = 1\n[workflow]\ngraph = \"workflow.fabro\"\n",
        )
        .unwrap();
        let user_settings_path = temp.path().join("settings.toml");
        fs::write(&user_settings_path, "_version = 1\n").unwrap();
        let sha = "0123456789abcdef0123456789abcdef01234567".to_string();

        let materialized = build_manifest_from_checkout(ManifestFromCheckoutInput {
            workflow:             "demo".to_string(),
            user_settings_path:   user_settings_path.clone(),
            checkout_dir:         checkout.clone(),
            target:               GitRunTarget {
                repo:   "workspace-org/app".to_string(),
                branch: "release".to_string(),
                tag:    Some("v1".to_string()),
                sha:    Some(sha.clone()),
            },
            environment_defaults: test_environment_defaults(),
        })
        .expect("manifest should build from checkout");

        assert_eq!(materialized.manifest.cwd, checkout.display().to_string());
        assert_eq!(
            materialized.manifest.target.path,
            ".fabro/workflows/demo/workflow.fabro"
        );
        assert!(
            materialized
                .manifest
                .configs
                .iter()
                .any(|config| config.path.as_deref() == Some(user_settings_path.to_str().unwrap()))
        );
        let git = materialized
            .manifest
            .git
            .as_ref()
            .expect("git context should be set");
        assert_eq!(git.origin_url, "https://github.com/workspace-org/app");
        assert_eq!(git.branch, "release");
        assert_eq!(git.sha.as_deref(), Some(sha.as_str()));
        assert_eq!(git.dirty, DirtyStatus::Clean);
        assert_eq!(materialized.target.tag.as_deref(), Some("v1"));
        assert_eq!(materialized.target.sha.as_deref(), Some(sha.as_str()));
        let submitted_manifest: serde_json::Value =
            serde_json::from_slice(&materialized.submitted_manifest_bytes)
                .expect("submitted bytes should be a manifest");
        assert!(submitted_manifest.get("run_id").is_none());
        assert_eq!(
            submitted_manifest,
            serde_json::to_value(&materialized.manifest).unwrap()
        );
    }
}
