use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use fabro_automation::AutomationId;
use fabro_manifest::WorkflowVersionCollectError;
use fabro_types::{
    GitHubRepositorySlug, GitRunTarget, RunId, RunIntent, RunIntentArgs, RunTarget,
    TargetValidationError, WorkflowVersionId,
};
use fabro_workflow_version::{WorkflowVersionStore, WorkflowVersionStoreError};
use tokio::{fs, task};

use crate::git_checkout::{
    GitCheckoutError, GitRepoCache, WorktreePrepareInput, resolve_git_auth_config,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AutomationRunMaterializeInput {
    pub automation_id: AutomationId,
    pub target:        GitRunTarget,
    pub workflow:      String,
    pub run_id:        RunId,
    pub temp_root:     PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct AutomationRunMaterialized {
    pub workflow_version_id: WorkflowVersionId,
    pub target:              GitRunTarget,
}

impl AutomationRunMaterialized {
    /// The admission request for an automation run: the packaged workflow
    /// version at the exact checked-out target, with no caller overrides.
    pub(crate) fn into_run_intent(self, environment_id: String) -> RunIntent {
        RunIntent {
            workflow_version_id: self.workflow_version_id,
            target:              RunTarget::Git(self.target),
            args:                RunIntentArgs::default(),
            environment_id:      Some(environment_id),
            parent_id:           None,
            title:               None,
            goal:                None,
        }
    }
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
    #[error("automation workflow was not found")]
    WorkflowNotFound {
        #[source]
        source: WorkflowVersionCollectError,
    },
    #[error("failed to package automation workflow versions")]
    Package {
        #[source]
        source: WorkflowVersionCollectError,
    },
    #[error("workflow-version packaging task failed")]
    PackageTask {
        #[source]
        source: task::JoinError,
    },
    #[error("failed to store automation workflow versions")]
    VersionStore {
        #[source]
        source: WorkflowVersionStoreError,
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
    github_credentials:  Option<fabro_github::GitHubCredentials>,
    github_api_base_url: String,
    http_client:         Option<fabro_http::HttpClient>,
    repo_cache:          Arc<GitRepoCache>,
    version_store:       WorkflowVersionStore,
}

impl ProductionAutomationRunMaterializer {
    pub(crate) fn new(
        github_credentials: Option<fabro_github::GitHubCredentials>,
        github_api_base_url: String,
        http_client: Option<fabro_http::HttpClient>,
        repo_cache: Arc<GitRepoCache>,
        version_store: WorkflowVersionStore,
    ) -> Self {
        Self {
            github_credentials,
            github_api_base_url,
            http_client,
            repo_cache,
            version_store,
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

        let workflow = PathBuf::from(input.workflow);
        let closure = task::spawn_blocking(move || {
            fabro_manifest::collect_workflow_versions(&workflow, &checkout_dir)
                .map_err(package_error)
        })
        .await
        .map_err(|source| RunMaterializeError::PackageTask { source })??;

        // Versions arrive dependency-first, and the store derives the same
        // content hash the collector did, so the root ID is known up front.
        for (expected, version) in closure.versions() {
            let stored = self
                .version_store
                .put(version)
                .await
                .map_err(|source| RunMaterializeError::VersionStore { source })?;
            debug_assert_eq!(
                stored, expected,
                "store and collector disagree on version ID"
            );
        }
        Ok(AutomationRunMaterialized {
            workflow_version_id: closure.root_id(),
            target:              exact_target,
        })
    }
}

fn package_error(error: WorkflowVersionCollectError) -> RunMaterializeError {
    if matches!(&error, WorkflowVersionCollectError::WorkflowNotFound { .. }) {
        RunMaterializeError::WorkflowNotFound { source: error }
    } else {
        RunMaterializeError::Package { source: error }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
pub struct TestAutomationRunMaterializer {
    inner:         std::sync::Arc<std::sync::Mutex<TestAutomationRunMaterializerState>>,
    version_store: Option<WorkflowVersionStore>,
}

#[cfg(any(test, feature = "test-support"))]
struct TestAutomationRunMaterializerState {
    captured_inputs: Vec<AutomationRunMaterializeInput>,
    response:        Result<Box<TestMaterializedWorkflow>, TargetValidationError>,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
struct TestMaterializedWorkflow {
    version: fabro_workflow_version::ValidatedWorkflowVersion,
    target:  GitRunTarget,
    store:   bool,
}

#[cfg(any(test, feature = "test-support"))]
impl TestAutomationRunMaterializer {
    pub fn succeed(target: GitRunTarget) -> Self {
        Self::new(Ok(Box::new(TestMaterializedWorkflow {
            version: test_workflow_version(),
            target,
            store: true,
        })))
    }

    pub fn return_unstored_version(target: GitRunTarget) -> Self {
        Self::new(Ok(Box::new(TestMaterializedWorkflow {
            version: test_workflow_version(),
            target,
            store: false,
        })))
    }

    pub fn fail_invalid_target() -> Self {
        Self::new(Err(TargetValidationError::Repository))
    }

    fn new(response: Result<Box<TestMaterializedWorkflow>, TargetValidationError>) -> Self {
        Self {
            inner:         std::sync::Arc::new(std::sync::Mutex::new(
                TestAutomationRunMaterializerState {
                    captured_inputs: Vec::new(),
                    response,
                },
            )),
            version_store: None,
        }
    }

    pub(crate) fn captured_inputs(&self) -> Vec<AutomationRunMaterializeInput> {
        self.inner
            .lock()
            .expect("test automation materializer lock poisoned")
            .captured_inputs
            .clone()
    }

    pub(crate) fn into_materializer(
        mut self,
        version_store: WorkflowVersionStore,
    ) -> std::sync::Arc<dyn AutomationRunMaterializer> {
        self.version_store = Some(version_store);
        std::sync::Arc::new(self)
    }
}

#[cfg(any(test, feature = "test-support"))]
fn test_workflow_version() -> fabro_workflow_version::ValidatedWorkflowVersion {
    use std::collections::BTreeMap;

    let entrypoint = fabro_types::WorkflowPath::new("workflow.fabro")
        .expect("test workflow entrypoint should be valid");
    let version = fabro_types::WorkflowVersion::new(
        entrypoint.clone(),
        BTreeMap::from([(
            entrypoint,
            "digraph Test { graph [goal=\"Test\"] start [shape=Mdiamond] exit [shape=Msquare] start -> exit }"
                .to_string(),
        )]),
        BTreeMap::new(),
    )
    .expect("test workflow version should have a valid shape");
    fabro_workflow_version::ValidatedWorkflowVersion::new(version)
        .expect("test workflow version should validate")
}

#[cfg(any(test, feature = "test-support"))]
#[async_trait]
impl AutomationRunMaterializer for TestAutomationRunMaterializer {
    async fn materialize(
        &self,
        input: AutomationRunMaterializeInput,
    ) -> Result<AutomationRunMaterialized, RunMaterializeError> {
        let response = {
            let mut guard = self
                .inner
                .lock()
                .expect("test automation materializer lock poisoned");
            guard.captured_inputs.push(input);
            guard.response.clone()
        };
        let materialized =
            *response.map_err(|source| RunMaterializeError::InvalidTarget { source })?;
        let store = self
            .version_store
            .as_ref()
            .expect("test materializer must be attached to a version store");
        let workflow_version_id = if materialized.store {
            store
                .put(&materialized.version)
                .await
                .map_err(|source| RunMaterializeError::VersionStore { source })?
        } else {
            materialized
                .version
                .version()
                .id()
                .expect("validated test workflow version should serialize canonically")
        };
        Ok(AutomationRunMaterialized {
            workflow_version_id,
            target: materialized.target,
        })
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::disallowed_methods,
        reason = "Materializer unit tests write small temporary workflow fixtures synchronously."
    )]

    use std::fs;
    use std::path::Path;
    use std::time::Duration;

    use object_store::memory::InMemory;
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn collected_closure_stores_dependency_first_and_idempotently() {
        let temp = TempDir::new().unwrap();
        let checkout = temp.path().join("checkout");
        let workflow_dir = checkout.join(".fabro/workflows/root");
        fs::create_dir_all(&workflow_dir).unwrap();
        fs::write(
            workflow_dir.join("workflow.toml"),
            "_version = 1\n[workflow]\ngraph = \"workflow.fabro\"\n",
        )
        .unwrap();
        fs::write(
            workflow_dir.join("workflow.fabro"),
            r#"digraph Root { child [stack.child_workflow="../child/workflow.fabro"] }"#,
        )
        .unwrap();
        let child_dir = checkout.join(".fabro/workflows/child");
        fs::create_dir_all(&child_dir).unwrap();
        fs::write(child_dir.join("workflow.fabro"), "digraph Child {}").unwrap();

        let closure =
            fabro_manifest::collect_workflow_versions(Path::new("root"), &checkout).unwrap();
        let database = fabro_store::test_support::test_database(
            Arc::new(InMemory::new()),
            "",
            Duration::from_millis(1),
            None,
        );
        let store = WorkflowVersionStore::new(database.blobs());

        for _ in 0..2 {
            for (expected, version) in closure.versions() {
                assert_eq!(store.put(version).await.unwrap(), expected);
            }
        }
        let loaded = store
            .get_closure(&closure.root_id())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.root_id(), closure.root_id());
        assert_eq!(loaded.versions().count(), 2);
    }
}
