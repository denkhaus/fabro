//! Sandboxes on a provider fabro does not bundle: any kind served by a
//! sandbox-driver plugin executable, and a bundled kind an operator chose to
//! run out of process.
//!
//! Fabro knows nothing about the provider beyond its declared capabilities,
//! so the environment maps onto the normalized [`SandboxSpec`] only: an
//! image or Dockerfile source when the environment names one (a host-style
//! provider gets a managed directory), resources, network policy, labels,
//! and environment variables. The provider chooses the working directory;
//! fabro lays its repository checkout out inside it.

use std::collections::BTreeMap;
use std::sync::Arc;

use fabro_github::GitHubCredentials;
use fabro_types::settings::run::{
    DockerfileSource, EnvironmentNetworkMode, RunCloneSettings, RunEnvironmentSettings,
};
use fabro_types::settings::server::ServerSandboxProviderSettings;
use fabro_types::{RunId, SandboxProviderKind};
use sandbox_driver::{
    Capabilities, NetworkPolicy, Resources, SandboxId, SandboxProvider, SandboxSource,
    SandboxSpec as DriverSpec,
};

use crate::driver::{ProviderConnectOptions, connect_provider};
use crate::driver_sandbox::{DriverSandbox, LayoutSource, RepoWorkspace};
use crate::managed_labels;

/// What an environment asks of a plugin provider.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PluginSandboxOptions {
    /// Image reference, when the environment names one.
    pub image:       Option<String>,
    /// Inline Dockerfile, when the environment names one instead of an
    /// image.
    pub dockerfile:  Option<String>,
    /// Environment variables for the sandbox, resolved.
    pub env:         BTreeMap<String, String>,
    pub network:     PluginNetwork,
    pub cpu_cores:   Option<u32>,
    pub memory_mb:   Option<u64>,
    pub disk_mb:     Option<u64>,
    /// Labels from the environment; fabro's managed labels are added.
    pub labels:      BTreeMap<String, String>,
    /// Maximum Git history depth fetched during clone; `None` fetches full
    /// history.
    pub clone_depth: Option<u32>,
    /// Create an empty workspace instead of cloning even when an origin
    /// exists.
    pub skip_clone:  bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PluginNetwork {
    #[default]
    ProviderDefault,
    AllowAll,
    Block,
    CidrAllowList(Vec<String>),
}

/// Map a resolved environment onto plugin options. `env` is the resolved
/// environment map (secrets substituted by the caller).
#[must_use]
pub fn plugin_options_from_environment(
    settings: &RunEnvironmentSettings,
    clone: &RunCloneSettings,
    env: BTreeMap<String, String>,
) -> PluginSandboxOptions {
    PluginSandboxOptions {
        image: settings.image.docker.clone(),
        dockerfile: match &settings.image.dockerfile {
            Some(DockerfileSource::Inline(content)) if settings.image.docker.is_none() => {
                Some(content.clone())
            }
            _ => None,
        },
        env,
        network: match settings.network.mode {
            EnvironmentNetworkMode::Block => PluginNetwork::Block,
            EnvironmentNetworkMode::AllowAll => PluginNetwork::AllowAll,
            EnvironmentNetworkMode::CidrAllowList => {
                PluginNetwork::CidrAllowList(settings.network.allow.clone())
            }
        },
        cpu_cores: settings
            .resources
            .cpu
            .and_then(|cpu| u32::try_from(cpu).ok()),
        memory_mb: settings
            .resources
            .memory
            .map(|size| size.as_bytes().div_ceil(1024 * 1024)),
        disk_mb: settings
            .resources
            .disk
            .map(|size| size.as_bytes().div_ceil(1024 * 1024)),
        labels: settings
            .labels
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        clone_depth: clone
            .depth_limit()
            .and_then(|depth| u32::try_from(depth).ok()),
        skip_clone: !clone.enabled,
    }
}

/// The driver spec for a fabro sandbox on a plugin provider.
pub(crate) fn driver_spec(options: &PluginSandboxOptions, run_id: Option<&RunId>) -> DriverSpec {
    let source = match (&options.image, &options.dockerfile) {
        (Some(reference), _) => SandboxSource::Image {
            reference: reference.clone(),
        },
        (None, Some(content)) => SandboxSource::Dockerfile {
            content: content.clone(),
        },
        // A provider without images (a host-style plugin) manages a
        // workspace directory of its own.
        (None, None) => SandboxSource::HostDirectory,
    };
    let mut spec = DriverSpec::new(source).network(match &options.network {
        PluginNetwork::ProviderDefault => NetworkPolicy::ProviderDefault,
        PluginNetwork::AllowAll => NetworkPolicy::AllowAll,
        PluginNetwork::Block => NetworkPolicy::Block,
        PluginNetwork::CidrAllowList(cidrs) => NetworkPolicy::CidrAllowList {
            cidrs: cidrs.clone(),
        },
    });
    if let Some(run_id) = run_id {
        spec = spec.name(format!("fabro-run-{run_id}"));
    }
    let user_labels: std::collections::HashMap<String, String> = options
        .labels
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let mut labels: Vec<(String, String)> =
        managed_labels::merge_for_run(Some(&user_labels), run_id)
            .into_iter()
            .collect();
    labels.sort();
    for (key, value) in labels {
        spec = spec.label(key, value);
    }
    for (key, value) in &options.env {
        spec = spec.env_var(key, value);
    }
    let mut resources = Resources::default();
    resources.cpu_cores = options.cpu_cores;
    resources.memory_mb = options.memory_mb;
    resources.disk_mb = options.disk_mb;
    spec.resources(resources)
}

/// The environment's default `allow_all` means "unrestricted", which a
/// provider without network controls already is; asking such a provider
/// for it explicitly would be rejected. An explicit restriction is still
/// requested, and refused by the provider when it cannot honor it.
fn supported_network(requested: NetworkPolicy, capabilities: &Capabilities) -> NetworkPolicy {
    match requested {
        NetworkPolicy::AllowAll if !capabilities.network.allow_all => {
            NetworkPolicy::ProviderDefault
        }
        other => other,
    }
}

async fn connect(
    kind: &SandboxProviderKind,
    settings: &ServerSandboxProviderSettings,
) -> crate::Result<Arc<dyn SandboxProvider>> {
    connect_provider(kind, settings, &ProviderConnectOptions::default())
        .await
        .map(|connected| connected.provider)
        .map_err(|error| {
            crate::Error::context(format!("Failed to connect to the {kind} provider"), error)
        })
}

/// A sandbox for a run on the plugin provider `kind`. The sandbox is
/// created by `initialize`; construction validates the clone request and
/// launches the plugin, so a bad spec or a missing executable fails first.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors SandboxSpec::Plugin; clone inputs are validated together"
)]
pub async fn plugin_sandbox(
    kind: SandboxProviderKind,
    settings: &ServerSandboxProviderSettings,
    options: PluginSandboxOptions,
    github_app: Option<&GitHubCredentials>,
    run_id: Option<RunId>,
    clone_origin_url: Option<String>,
    clone_branch: Option<String>,
    clone_tag: Option<String>,
    clone_commit_sha: Option<String>,
) -> crate::Result<DriverSandbox> {
    let workspace = RepoWorkspace::plan(
        LayoutSource::ProviderWorkingDirectory,
        options.skip_clone,
        clone_origin_url.as_deref(),
        clone_branch.as_deref(),
        clone_tag.as_deref(),
        clone_commit_sha.as_deref(),
        options.clone_depth,
        github_app,
    )?;
    let provider = connect(&kind, settings).await?;
    let mut spec = driver_spec(&options, run_id.as_ref());
    spec.network = supported_network(spec.network, provider.capabilities());
    Ok(DriverSandbox::pending(
        kind,
        provider,
        spec,
        options.image.clone(),
        workspace,
    ))
}

/// Reattach to a run's sandbox on the plugin provider `kind` by its
/// persisted id. The sandbox must carry fabro's labels.
pub async fn attach_plugin(
    kind: SandboxProviderKind,
    settings: &ServerSandboxProviderSettings,
    sandbox_id: &str,
    repo_cloned: bool,
    working_directory: String,
    clone_origin_url: Option<String>,
    run_id: Option<RunId>,
) -> crate::Result<DriverSandbox> {
    let provider = connect(&kind, settings).await?;
    let id = SandboxId::try_new(sandbox_id)
        .map_err(|error| crate::Error::context(format!("Invalid {kind} sandbox id"), error))?;
    let handle = provider.attach(&id, None).await.map_err(|error| {
        crate::Error::context(
            format!("Failed to reconnect {kind} sandbox '{sandbox_id}'"),
            error,
        )
    })?;
    let status = handle.describe().await?;
    managed_labels::verify_managed(&kind, sandbox_id, &status.labels, run_id.as_ref())?;
    let workspace = RepoWorkspace::attached(
        LayoutSource::ProviderWorkingDirectory,
        repo_cloned,
        working_directory,
        clone_origin_url,
    );
    Ok(DriverSandbox::attached(kind, handle, workspace))
}

#[cfg(test)]
mod tests {
    use fabro_types::settings::run::{
        EnvironmentImageSettings, EnvironmentLifecycleSettings, EnvironmentNetworkSettings,
        EnvironmentResourcesSettings,
    };

    use super::*;

    fn environment(kind: &str) -> RunEnvironmentSettings {
        RunEnvironmentSettings {
            id:        kind.to_string(),
            provider:  SandboxProviderKind::try_new(kind).unwrap(),
            cwd:       None,
            image:     EnvironmentImageSettings::default(),
            resources: EnvironmentResourcesSettings::default(),
            network:   EnvironmentNetworkSettings::default(),
            lifecycle: EnvironmentLifecycleSettings::default(),
            labels:    std::collections::HashMap::from([(
                "team".to_string(),
                "platform".to_string(),
            )]),
            env:       std::collections::HashMap::new(),
        }
    }

    #[test]
    fn options_without_an_image_ask_for_a_managed_directory() {
        let options = plugin_options_from_environment(
            &environment("host"),
            &RunCloneSettings::default(),
            BTreeMap::from([("FOO".to_string(), "bar".to_string())]),
        );
        assert!(options.image.is_none());
        assert_eq!(options.clone_depth, Some(100));
        assert!(!options.skip_clone);

        let run_id: RunId = "01HY0000000000000000000000".parse().unwrap();
        let spec = driver_spec(&options, Some(&run_id));
        assert!(matches!(spec.source, SandboxSource::HostDirectory));
        assert!(spec.working_directory.is_none());
        assert_eq!(
            spec.name.as_deref(),
            Some("fabro-run-01HY0000000000000000000000")
        );
        assert_eq!(spec.env.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(
            spec.labels.get("team").map(String::as_str),
            Some("platform")
        );
        assert_eq!(
            spec.labels.get("sh.fabro.managed").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            spec.labels.get("sh.fabro.run_id").map(String::as_str),
            Some("01HY0000000000000000000000")
        );
        assert!(matches!(spec.network, NetworkPolicy::AllowAll));
    }

    #[test]
    fn allow_all_falls_back_to_the_provider_default_without_network_control() {
        let none = Capabilities::minimal(sandbox_driver::Isolation::None);
        assert!(matches!(
            supported_network(NetworkPolicy::AllowAll, &none),
            NetworkPolicy::ProviderDefault
        ));
        assert!(matches!(
            supported_network(NetworkPolicy::Block, &none),
            NetworkPolicy::Block
        ));
        let mut full = Capabilities::minimal(sandbox_driver::Isolation::Container);
        full.network.allow_all = true;
        assert!(matches!(
            supported_network(NetworkPolicy::AllowAll, &full),
            NetworkPolicy::AllowAll
        ));
    }

    #[test]
    fn options_with_an_image_map_resources_and_network() {
        let mut settings = environment("e2b");
        settings.image.docker = Some("ubuntu:24.04".to_string());
        settings.resources.cpu = Some(2);
        settings.network.mode = EnvironmentNetworkMode::Block;
        let clone = RunCloneSettings {
            enabled: false,
            depth:   0,
        };
        let options = plugin_options_from_environment(&settings, &clone, BTreeMap::new());
        assert_eq!(options.image.as_deref(), Some("ubuntu:24.04"));
        assert!(options.skip_clone);
        assert_eq!(options.clone_depth, None);

        let spec = driver_spec(&options, None);
        assert!(matches!(
            &spec.source,
            SandboxSource::Image { reference } if reference == "ubuntu:24.04"
        ));
        assert_eq!(spec.resources.cpu_cores, Some(2));
        assert!(matches!(spec.network, NetworkPolicy::Block));
        assert!(spec.name.is_none());
    }
}
