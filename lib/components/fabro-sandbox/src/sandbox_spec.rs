use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use fabro_github::GitHubCredentials;
use fabro_types::settings::server::ServerSandboxProviderSettings;
use fabro_types::{RunId, RunSandboxInstance, RunSandboxRuntime, SandboxProviderKind};

use crate::daytona::{self, DaytonaConfig};
use crate::docker::{self, DockerSandboxOptions};
use crate::driver::DaytonaCredentials;
use crate::driver_sandbox::local_sandbox;
use crate::plugin::{self, PluginSandboxOptions};
use crate::{Sandbox, SandboxEventCallback, clone_source};

/// Options for sandbox initialization and construction.
pub enum SandboxSpec {
    Local {
        working_directory: PathBuf,
    },
    Docker {
        config:           DockerSandboxOptions,
        github_app:       Option<GitHubCredentials>,
        run_id:           Option<RunId>,
        clone_origin_url: Option<String>,
        clone_branch:     Option<String>,
        clone_tag:        Option<String>,
        clone_commit_sha: Option<String>,
    },
    Daytona {
        config:           Box<DaytonaConfig>,
        github_app:       Option<GitHubCredentials>,
        run_id:           Option<RunId>,
        clone_origin_url: Option<String>,
        clone_branch:     Option<String>,
        clone_tag:        Option<String>,
        clone_commit_sha: Option<String>,
        /// Vault credentials for the Daytona control plane; `None` fails at
        /// build time with a clear message rather than reading the process
        /// environment.
        credentials:      Option<DaytonaCredentials>,
    },
    /// A provider served by a sandbox-driver plugin executable.
    Plugin {
        kind:             SandboxProviderKind,
        settings:         Box<ServerSandboxProviderSettings>,
        options:          Box<PluginSandboxOptions>,
        github_app:       Option<GitHubCredentials>,
        run_id:           Option<RunId>,
        clone_origin_url: Option<String>,
        clone_branch:     Option<String>,
        clone_tag:        Option<String>,
        clone_commit_sha: Option<String>,
    },
}

impl SandboxSpec {
    pub fn provider(&self) -> SandboxProviderKind {
        match self {
            Self::Local { .. } => SandboxProviderKind::LOCAL,
            Self::Docker { .. } => SandboxProviderKind::DOCKER,
            Self::Daytona { .. } => SandboxProviderKind::DAYTONA,
            Self::Plugin { kind, .. } => kind.clone(),
        }
    }

    pub fn provider_name(&self) -> String {
        self.provider().to_string()
    }

    /// Build initialized sandbox metadata for persistence.
    pub fn to_run_sandbox_instance(
        &self,
        sandbox: &dyn Sandbox,
        run_id: RunId,
    ) -> RunSandboxInstance {
        let working_directory = sandbox.working_directory().to_string();
        let id = {
            let info = sandbox.sandbox_info();
            if info.is_empty() {
                format!("local:{run_id}")
            } else {
                info
            }
        };

        match self {
            Self::Docker {
                config,
                clone_origin_url,
                clone_branch,
                ..
            } => {
                let repo_cloned = clone_source::repo_cloned_for_record(
                    config.skip_clone,
                    clone_origin_url.as_deref(),
                );
                let layout = runtime_layout_metadata(
                    repo_cloned,
                    clone_origin_url.as_deref(),
                    docker::WORKING_DIRECTORY,
                    docker::REPOS_ROOT,
                );
                RunSandboxInstance {
                    provider: self.provider(),
                    image:    (!config.image.is_empty()).then(|| config.image.clone()),
                    snapshot: None,
                    runtime:  RunSandboxRuntime {
                        id,
                        working_directory: working_directory.clone(),
                        repo_cloned,
                        clone_origin_url: clone_source::clean_clone_origin_for_record(
                            clone_origin_url.as_deref(),
                        ),
                        clone_branch: clone_branch.clone(),
                        workspace_root: Some(docker::WORKING_DIRECTORY.to_string()),
                        repos_root: Some(docker::REPOS_ROOT.to_string()),
                        primary_repo_path: layout
                            .as_ref()
                            .map(|layout| layout.primary_repo_path.clone()),
                        primary_repo_link: layout
                            .as_ref()
                            .map(|layout| layout.primary_repo_link.clone()),
                    },
                }
            }
            Self::Daytona {
                config,
                clone_origin_url,
                clone_branch,
                ..
            } => {
                let repo_cloned = clone_source::repo_cloned_for_record(
                    config.skip_clone,
                    clone_origin_url.as_deref(),
                );
                let layout = runtime_layout_metadata(
                    repo_cloned,
                    clone_origin_url.as_deref(),
                    daytona::WORKING_DIRECTORY,
                    daytona::REPOS_ROOT,
                );
                RunSandboxInstance {
                    provider: self.provider(),
                    image:    None,
                    snapshot: sandbox.snapshot_info(),
                    runtime:  RunSandboxRuntime {
                        id,
                        working_directory: working_directory.clone(),
                        repo_cloned,
                        clone_origin_url: clone_source::clean_clone_origin_for_record(
                            clone_origin_url.as_deref(),
                        ),
                        clone_branch: clone_branch.clone(),
                        workspace_root: Some(daytona::WORKING_DIRECTORY.to_string()),
                        repos_root: Some(daytona::REPOS_ROOT.to_string()),
                        primary_repo_path: layout
                            .as_ref()
                            .map(|layout| layout.primary_repo_path.clone()),
                        primary_repo_link: layout
                            .as_ref()
                            .map(|layout| layout.primary_repo_link.clone()),
                    },
                }
            }
            Self::Plugin {
                options,
                clone_origin_url,
                clone_branch,
                ..
            } => {
                let repo_cloned = clone_source::repo_cloned_for_record(
                    options.skip_clone,
                    clone_origin_url.as_deref(),
                );
                let layout = sandbox.workspace_layout();
                RunSandboxInstance {
                    provider: self.provider(),
                    image:    options.image.clone(),
                    snapshot: sandbox.snapshot_info(),
                    runtime:  RunSandboxRuntime {
                        id,
                        working_directory,
                        repo_cloned,
                        clone_origin_url: clone_source::clean_clone_origin_for_record(
                            clone_origin_url.as_deref(),
                        ),
                        clone_branch: clone_branch.clone(),
                        workspace_root: layout.as_ref().map(|layout| layout.workspace_root.clone()),
                        repos_root: layout.as_ref().map(|layout| layout.repos_root.clone()),
                        primary_repo_path: layout
                            .as_ref()
                            .and_then(|layout| layout.primary_repo_path.clone()),
                        primary_repo_link: layout
                            .as_ref()
                            .and_then(|layout| layout.primary_repo_link.clone()),
                    },
                }
            }
            Self::Local { .. } => RunSandboxInstance {
                provider: self.provider(),
                image:    None,
                snapshot: None,
                runtime:  RunSandboxRuntime {
                    id,
                    working_directory,
                    repo_cloned: None,
                    clone_origin_url: None,
                    clone_branch: None,
                    workspace_root: None,
                    repos_root: None,
                    primary_repo_path: None,
                    primary_repo_link: None,
                },
            },
        }
    }

    pub async fn build(
        &self,
        event_callback: Option<SandboxEventCallback>,
    ) -> Result<Arc<dyn Sandbox>, anyhow::Error> {
        match self {
            Self::Local { working_directory } => {
                let mut sandbox = local_sandbox(working_directory.clone())
                    .await
                    .context("Failed to create local sandbox")?;
                if let Some(callback) = event_callback {
                    sandbox.set_event_callback(callback);
                }
                Ok(Arc::new(sandbox))
            }
            Self::Docker {
                config,
                github_app,
                run_id,
                clone_origin_url,
                clone_branch,
                clone_tag,
                clone_commit_sha,
            } => {
                let mut sandbox = docker::docker_sandbox(
                    config.clone(),
                    github_app.as_ref(),
                    *run_id,
                    clone_origin_url.clone(),
                    clone_branch.clone(),
                    clone_tag.clone(),
                    clone_commit_sha.clone(),
                )
                .await
                .context("Failed to create Docker sandbox")?;
                if let Some(callback) = event_callback {
                    sandbox.set_event_callback(callback);
                }
                Ok(Arc::new(sandbox))
            }
            Self::Daytona {
                config,
                github_app,
                run_id,
                clone_origin_url,
                clone_branch,
                clone_tag,
                clone_commit_sha,
                credentials,
            } => {
                let credentials = credentials.as_ref().context(
                    "Daytona sandboxes require DAYTONA_API_KEY in the vault; run `fabro secret set DAYTONA_API_KEY`",
                )?;
                let mut sandbox = daytona::daytona_sandbox(
                    config.as_ref().clone(),
                    github_app.as_ref(),
                    *run_id,
                    clone_origin_url.clone(),
                    clone_branch.clone(),
                    clone_tag.clone(),
                    clone_commit_sha.clone(),
                    credentials,
                )
                .await
                .context("Failed to create Daytona sandbox")?;
                if let Some(callback) = event_callback {
                    sandbox.set_event_callback(callback);
                }
                Ok(Arc::new(sandbox))
            }
            Self::Plugin {
                kind,
                settings,
                options,
                github_app,
                run_id,
                clone_origin_url,
                clone_branch,
                clone_tag,
                clone_commit_sha,
            } => {
                let mut sandbox = plugin::plugin_sandbox(
                    kind.clone(),
                    settings,
                    options.as_ref().clone(),
                    github_app.as_ref(),
                    *run_id,
                    clone_origin_url.clone(),
                    clone_branch.clone(),
                    clone_tag.clone(),
                    clone_commit_sha.clone(),
                )
                .await
                .with_context(|| format!("Failed to create {kind} sandbox"))?;
                if let Some(callback) = event_callback {
                    sandbox.set_event_callback(callback);
                }
                Ok(Arc::new(sandbox))
            }
        }
    }
}

fn runtime_layout_metadata(
    repo_cloned: Option<bool>,
    clone_origin_url: Option<&str>,
    workspace_root: &str,
    repos_root: &str,
) -> Option<clone_source::GitHubRepoLayout> {
    if repo_cloned != Some(true) {
        return None;
    }
    clone_source::github_repo_layout(clone_origin_url?, workspace_root, repos_root).ok()
}

#[cfg(test)]
mod tests {
    use fabro_types::RunId;

    use super::*;
    use crate::test_support::MockSandbox;

    #[test]
    fn docker_run_sandbox_persists_layout_metadata_for_cloned_repo() {
        let spec = SandboxSpec::Docker {
            config:           DockerSandboxOptions::default(),
            github_app:       None,
            run_id:           None,
            clone_origin_url: Some("git@github.com:brynary/rack-test.git".to_string()),
            clone_branch:     Some("main".to_string()),
            clone_tag:        None,
            clone_commit_sha: None,
        };
        let mut sandbox = MockSandbox::linux();
        sandbox.working_dir = "/workspace/rack-test";

        let run_id: RunId = "01HY0000000000000000000000".parse().unwrap();
        let record = spec.to_run_sandbox_instance(&sandbox, run_id);
        let runtime = record.runtime;

        assert_eq!(runtime.working_directory, "/workspace/rack-test");
        assert_eq!(runtime.repo_cloned, Some(true));
        assert_eq!(
            runtime.clone_origin_url.as_deref(),
            Some("https://github.com/brynary/rack-test")
        );
        assert_eq!(runtime.workspace_root.as_deref(), Some("/workspace"));
        assert_eq!(runtime.repos_root.as_deref(), Some("/repos"));
        assert_eq!(
            runtime.primary_repo_path.as_deref(),
            Some("/repos/brynary/rack-test")
        );
        assert_eq!(
            runtime.primary_repo_link.as_deref(),
            Some("/workspace/rack-test")
        );
        let runtime_json = serde_json::to_value(&runtime).expect("runtime should serialize");
        assert!(runtime_json.get("clone_commit_sha").is_none());
    }

    #[tokio::test]
    async fn invalid_exact_checkout_spec_fails_before_provider_connection() {
        let spec = SandboxSpec::Docker {
            config:           DockerSandboxOptions::default(),
            github_app:       None,
            run_id:           None,
            clone_origin_url: Some("https://github.com/acme/widgets".to_string()),
            clone_branch:     Some("main".to_string()),
            clone_tag:        None,
            clone_commit_sha: Some("not-a-sha".to_string()),
        };

        let error = spec
            .build(None)
            .await
            .err()
            .expect("spec validation should run before Docker connection");
        assert!(
            error
                .to_string()
                .contains("Failed to create Docker sandbox")
        );
        assert!(format!("{error:#}").contains("40 ASCII hexadecimal"));
        assert!(!format!("{error:#}").contains("Docker daemon"));
    }

    #[test]
    fn docker_run_sandbox_omits_primary_repo_metadata_for_empty_workspace() {
        let spec = SandboxSpec::Docker {
            config:           DockerSandboxOptions {
                skip_clone: true,
                ..DockerSandboxOptions::default()
            },
            github_app:       None,
            run_id:           None,
            clone_origin_url: Some("https://gitlab.com/acme/widgets".to_string()),
            clone_branch:     None,
            clone_tag:        None,
            clone_commit_sha: None,
        };
        let mut sandbox = MockSandbox::linux();
        sandbox.working_dir = "/workspace";

        let run_id: RunId = "01HY0000000000000000000000".parse().unwrap();
        let record = spec.to_run_sandbox_instance(&sandbox, run_id);
        let runtime = record.runtime;

        assert_eq!(runtime.working_directory, "/workspace");
        assert_eq!(runtime.repo_cloned, Some(false));
        assert_eq!(runtime.workspace_root.as_deref(), Some("/workspace"));
        assert_eq!(runtime.repos_root.as_deref(), Some("/repos"));
        assert!(runtime.primary_repo_path.is_none());
        assert!(runtime.primary_repo_link.is_none());
    }
}
