//! The `docker` provider kind: fabro's environment mapping onto the
//! sandbox-driver Docker provider.
//!
//! Fabro decides the image, resources, network policy, environment, labels,
//! and workspace layout; the driver creates and drives the container. The
//! container's working directory is [`WORKING_DIRECTORY`]; a cloned
//! repository checks out under [`REPOS_ROOT`] and is linked into the
//! workspace, so the run works in `/workspace/<repo>`.

use std::collections::BTreeMap;

use fabro_github::GitHubCredentials;
use fabro_types::settings::run::RunCloneSettings;
use fabro_types::settings::server::ServerSandboxProviderSettings;
use fabro_types::{RunId, SandboxProviderKind};
use sandbox_driver::{
    HealthStatus, NetworkPolicy, Resources, SandboxId, SandboxSource, SandboxSpec as DriverSpec,
};
use sandbox_driver_docker_config::DockerProviderConfig;

use crate::driver::{ProviderConnectOptions, connect_provider};
use crate::driver_sandbox::{DriverSandbox, RepoWorkspace, WorkspaceLayout};
use crate::managed_labels::{self, MANAGED_LABEL, MANAGED_LABEL_VALUE, RUN_ID_LABEL};

pub const WORKING_DIRECTORY: &str = "/workspace";
pub const REPOS_ROOT: &str = "/repos";
const DEFAULT_GIT_CLONE_DEPTH: usize = RunCloneSettings::DEFAULT_DEPTH.unsigned_abs() as usize;
/// Docker's default CFS period; a whole core is one period's worth of quota.
const CPU_PERIOD_MICROS: i64 = 100_000;

/// Options fabro derives from an environment for a Docker sandbox.
#[derive(Clone, Debug, PartialEq)]
pub struct DockerSandboxOptions {
    /// Docker image to use.
    pub image:        String,
    /// Docker network mode. Default: `Some("bridge")`; `Some("none")` blocks.
    pub network_mode: Option<String>,
    /// Memory limit in bytes. `None` = unlimited.
    pub memory_limit: Option<i64>,
    /// CPU quota (microseconds per 100ms period). `None` = unlimited.
    pub cpu_quota:    Option<i64>,
    /// Whether to pull the image if not found locally. Default: `true`.
    pub auto_pull:    bool,
    /// Additional `KEY=VALUE` environment variables for the container.
    pub env_vars:     Vec<String>,
    /// Maximum Git history depth fetched during clone; `None` fetches full
    /// history.
    pub clone_depth:  Option<usize>,
    /// Create an empty workspace instead of cloning even when an origin exists.
    pub skip_clone:   bool,
}

impl Default for DockerSandboxOptions {
    fn default() -> Self {
        Self {
            image:        "buildpack-deps:noble".to_string(),
            network_mode: Some("bridge".to_string()),
            memory_limit: None,
            cpu_quota:    None,
            auto_pull:    true,
            env_vars:     Vec::new(),
            clone_depth:  Some(DEFAULT_GIT_CLONE_DEPTH),
            skip_clone:   false,
        }
    }
}

/// The workspace layout every Docker sandbox uses.
pub(crate) fn layout() -> WorkspaceLayout {
    WorkspaceLayout {
        workspace_root: WORKING_DIRECTORY.to_string(),
        repos_root:     REPOS_ROOT.to_string(),
    }
}

pub(crate) fn container_name(run_id: &RunId) -> String {
    format!("fabro-run-{run_id}")
}

/// The driver spec for a fabro Docker sandbox.
pub(crate) fn driver_spec(options: &DockerSandboxOptions, run_id: Option<&RunId>) -> DriverSpec {
    let mut spec = DriverSpec::new(SandboxSource::Image {
        reference: options.image.clone(),
    })
    .working_directory(WORKING_DIRECTORY)
    .network(match options.network_mode.as_deref() {
        Some("none") => NetworkPolicy::Block,
        _ => NetworkPolicy::AllowAll,
    })
    .provider_config(
        DockerProviderConfig {
            auto_pull: options.auto_pull,
            ..DockerProviderConfig::default()
        }
        .into_value(),
    );
    if let Some(run_id) = run_id {
        spec = spec.name(container_name(run_id));
    }
    for (key, value) in managed_labels::for_run(run_id) {
        spec = spec.label(key, value);
    }
    for entry in &options.env_vars {
        let (key, value) = entry.split_once('=').unwrap_or((entry.as_str(), ""));
        spec = spec.env_var(key, value);
    }
    let mut resources = Resources::default();
    resources.cpu_cores = options
        .cpu_quota
        .filter(|quota| *quota > 0)
        .map(|quota| (quota + CPU_PERIOD_MICROS - 1) / CPU_PERIOD_MICROS)
        .and_then(|cores| u32::try_from(cores).ok());
    resources.memory_mb = options
        .memory_limit
        .filter(|bytes| *bytes > 0)
        .and_then(|bytes| u64::try_from(bytes).ok())
        .map(|bytes| bytes.div_ceil(1024 * 1024));
    spec.resources(resources)
}

async fn connect_docker() -> crate::Result<std::sync::Arc<dyn sandbox_driver::SandboxProvider>> {
    connect_provider(
        &SandboxProviderKind::DOCKER,
        &ServerSandboxProviderSettings::default(),
        &ProviderConnectOptions::default(),
    )
    .await
    .map(|connected| connected.provider)
    .map_err(|error| crate::Error::context("Failed to connect to the Docker provider", error))
}

/// A Docker sandbox for a run. The container is created by `initialize`;
/// construction validates the clone request and connects the provider, so a
/// bad spec fails before any daemon call.
pub async fn docker_sandbox(
    options: DockerSandboxOptions,
    github_app: Option<&GitHubCredentials>,
    run_id: Option<RunId>,
    clone_origin_url: Option<String>,
    clone_branch: Option<String>,
    clone_tag: Option<String>,
    clone_commit_sha: Option<String>,
) -> crate::Result<DriverSandbox> {
    let workspace = RepoWorkspace::plan(
        layout(),
        options.skip_clone,
        clone_origin_url.as_deref(),
        clone_branch.as_deref(),
        clone_tag.as_deref(),
        clone_commit_sha.as_deref(),
        options
            .clone_depth
            .and_then(|depth| u32::try_from(depth).ok()),
        github_app,
    )?;
    let provider = connect_docker().await?;
    let spec = driver_spec(&options, run_id.as_ref());
    Ok(DriverSandbox::pending(
        SandboxProviderKind::DOCKER,
        provider,
        spec,
        Some(options.image),
        workspace,
    ))
}

/// Reattach to a run's Docker container by its persisted id.
///
/// The container must carry fabro's managed label and, when a run id is
/// known, the matching run label: the driver shares a daemon with every
/// other application, and fabro never operates on a container it did not
/// create.
pub async fn attach_docker(
    container_id: &str,
    repo_cloned: bool,
    working_directory: String,
    clone_origin_url: Option<String>,
    run_id: Option<RunId>,
) -> crate::Result<DriverSandbox> {
    let provider = connect_docker().await?;
    let id = SandboxId::try_new(container_id)
        .map_err(|error| crate::Error::context("Invalid Docker container id", error))?;
    let handle = provider.attach(&id, None).await.map_err(|error| {
        crate::Error::context(
            format!("Failed to reconnect Docker container '{container_id}'"),
            error,
        )
    })?;
    let status = handle.describe().await?;
    verify_managed_labels(container_id, &status.labels, run_id.as_ref())?;
    let workspace =
        RepoWorkspace::attached(layout(), repo_cloned, working_directory, clone_origin_url);
    Ok(DriverSandbox::attached(
        SandboxProviderKind::DOCKER,
        handle,
        workspace,
    ))
}

/// Whether the Docker daemon answers. Used by `fabro doctor`.
pub async fn check_docker_daemon() -> crate::Result<()> {
    let provider = connect_docker().await?;
    let health = provider
        .health()
        .await
        .map_err(|error| crate::Error::context("Docker health check failed", error))?;
    match health.status {
        HealthStatus::Ok | HealthStatus::Unknown => Ok(()),
        HealthStatus::Unreachable | HealthStatus::Unauthorized => {
            Err(crate::Error::message(health.message.unwrap_or_else(|| {
                "Failed to reach Docker daemon".to_string()
            })))
        }
        _ => Err(crate::Error::message(
            "Docker daemon reported an unknown health state",
        )),
    }
}

pub(crate) fn verify_managed_labels(
    container_id: &str,
    labels: &BTreeMap<String, String>,
    run_id: Option<&RunId>,
) -> crate::Result<()> {
    if labels.get(MANAGED_LABEL).map(String::as_str) != Some(MANAGED_LABEL_VALUE) {
        return Err(crate::Error::message(format!(
            "Refusing to operate on Docker container '{container_id}' because it is missing label {MANAGED_LABEL}={MANAGED_LABEL_VALUE}"
        )));
    }
    if let Some(run_id) = run_id {
        let actual = labels.get(RUN_ID_LABEL).map(String::as_str);
        let expected = run_id.to_string();
        if actual != Some(expected.as_str()) {
            return Err(crate::Error::message(format!(
                "Refusing to operate on Docker container '{container_id}' because label {RUN_ID_LABEL}={actual:?} does not match run {run_id}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_spec_maps_image_workspace_labels_env_and_limits() {
        let run_id: RunId = "01HY0000000000000000000000".parse().unwrap();
        let options = DockerSandboxOptions {
            env_vars: vec![
                "FOO=bar".to_string(),
                "BASH_ENV=/tmp/untrusted-startup".to_string(),
            ],
            memory_limit: Some(4_000_000_000),
            cpu_quota: Some(200_000),
            network_mode: Some("none".to_string()),
            auto_pull: false,
            ..DockerSandboxOptions::default()
        };
        let spec = driver_spec(&options, Some(&run_id));

        assert!(matches!(
            &spec.source,
            SandboxSource::Image { reference } if reference == "buildpack-deps:noble"
        ));
        assert_eq!(
            spec.name.as_deref(),
            Some("fabro-run-01HY0000000000000000000000")
        );
        assert_eq!(spec.working_directory.as_deref(), Some(WORKING_DIRECTORY));
        assert_eq!(
            spec.labels.get(MANAGED_LABEL).map(String::as_str),
            Some("true")
        );
        assert_eq!(
            spec.labels.get(RUN_ID_LABEL).map(String::as_str),
            Some("01HY0000000000000000000000")
        );
        assert_eq!(spec.env.get("FOO").map(String::as_str), Some("bar"));
        // The driver blanks BASH_ENV on every container; a caller value is
        // passed through here and overridden there.
        assert_eq!(spec.resources.cpu_cores, Some(2));
        assert_eq!(spec.resources.memory_mb, Some(3815));
        assert!(matches!(spec.network, NetworkPolicy::Block));
        assert_eq!(spec.provider_config["auto_pull"], false);
    }

    #[test]
    fn driver_spec_defaults_to_bridge_networking_without_a_name() {
        let spec = driver_spec(&DockerSandboxOptions::default(), None);
        assert!(spec.name.is_none());
        assert!(matches!(spec.network, NetworkPolicy::AllowAll));
        assert_eq!(spec.resources, Resources::default());
        assert!(!spec.labels.contains_key(RUN_ID_LABEL));
    }

    #[test]
    fn managed_label_check_requires_fabro_ownership_and_matching_run() {
        let run_id: RunId = "01HY0000000000000000000000".parse().unwrap();
        let mut labels = BTreeMap::new();
        assert!(verify_managed_labels("c1", &labels, None).is_err());
        labels.insert(MANAGED_LABEL.to_string(), "true".to_string());
        assert!(verify_managed_labels("c1", &labels, None).is_ok());
        assert!(verify_managed_labels("c1", &labels, Some(&run_id)).is_err());
        labels.insert(RUN_ID_LABEL.to_string(), run_id.to_string());
        assert!(verify_managed_labels("c1", &labels, Some(&run_id)).is_ok());
    }
}
