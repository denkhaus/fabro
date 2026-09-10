//! What an environment asks of a sandbox, mapped once for every provider.
//!
//! The environment names an image or Dockerfile, resources, a network
//! policy, labels, variables, a lifecycle, and a clone policy. Every
//! provider consumes the same [`SandboxOptions`]: the driver spec is built
//! from them in one place, and a bundled provider adds only what its
//! backend needs on top (the Docker working directory, the Daytona
//! snapshot and timers) in its own overlay.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use fabro_types::RunId;
use fabro_types::settings::run::{
    DockerfileSource, EnvironmentNetworkMode, RunCloneSettings, RunEnvironmentSettings,
};
use sandbox_driver::{
    Capabilities, NetworkPolicy, Resources, SandboxSource, SandboxSpec as DriverSpec,
};

use crate::managed_labels;

/// What an environment asks of a sandbox, provider-neutral.
#[derive(Clone, Debug, Default)]
pub struct SandboxOptions {
    /// Image reference, when the environment names one.
    pub image:        Option<String>,
    /// Inline Dockerfile, when the environment names one instead of an
    /// image.
    pub dockerfile:   Option<String>,
    /// Environment variables for the sandbox, resolved.
    pub env:          BTreeMap<String, String>,
    pub network:      NetworkPolicy,
    pub cpu:          Option<u32>,
    pub memory_bytes: Option<u64>,
    pub disk_bytes:   Option<u64>,
    /// Labels from the environment; fabro's managed labels are added.
    pub labels:       BTreeMap<String, String>,
    /// Idle time before the provider stops the sandbox, when the
    /// environment sets one.
    pub auto_stop:    Option<Duration>,
    /// Maximum Git history depth fetched during clone; `None` fetches full
    /// history.
    pub clone_depth:  Option<u32>,
    /// Create an empty workspace instead of cloning even when an origin
    /// exists.
    pub skip_clone:   bool,
}

impl SandboxOptions {
    /// Memory in whole mebibytes, rounded up.
    pub fn memory_mb(&self) -> Option<u64> {
        self.memory_bytes.map(|bytes| bytes.div_ceil(1024 * 1024))
    }

    /// Disk in whole mebibytes, rounded up.
    pub fn disk_mb(&self) -> Option<u64> {
        self.disk_bytes.map(|bytes| bytes.div_ceil(1024 * 1024))
    }
}

/// Maps resolved environment settings onto sandbox options. `env` is the
/// environment's variables, resolved by the caller: the worker resolves
/// secrets through the vault, while preflight carries them in source form.
///
/// A Dockerfile given as a path must have been resolved to inline content
/// earlier; none of the providers can read a path.
pub fn options_from_environment(
    settings: &RunEnvironmentSettings,
    clone: &RunCloneSettings,
    env: BTreeMap<String, String>,
) -> crate::Result<SandboxOptions> {
    // fabro-config rejects environments that set both image.docker and
    // image.dockerfile. If both still arrive here, the image wins.
    let dockerfile = match (&settings.image.docker, &settings.image.dockerfile) {
        (Some(_), _) | (None, None) => None,
        (None, Some(DockerfileSource::Inline(content))) => Some(content.clone()),
        (None, Some(DockerfileSource::Path { path })) => {
            return Err(crate::Error::message(format!(
                "environment `{}` names a Dockerfile path ({path}) that should have been \
                 resolved to inline content before sandbox creation",
                settings.id
            )));
        }
    };
    Ok(SandboxOptions {
        image: settings.image.docker.clone(),
        dockerfile,
        env,
        network: match settings.network.mode {
            EnvironmentNetworkMode::Block => NetworkPolicy::Block,
            EnvironmentNetworkMode::AllowAll => NetworkPolicy::AllowAll,
            EnvironmentNetworkMode::CidrAllowList => NetworkPolicy::CidrAllowList {
                cidrs: settings.network.allow.clone(),
            },
        },
        cpu: settings
            .resources
            .cpu
            .and_then(|cpu| u32::try_from(cpu).ok()),
        memory_bytes: settings.resources.memory.map(|size| size.as_bytes()),
        disk_bytes: settings.resources.disk.map(|size| size.as_bytes()),
        labels: settings
            .labels
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
        auto_stop: settings
            .lifecycle
            .auto_stop
            .map(|duration| duration.as_std()),
        clone_depth: clone
            .depth_limit()
            .and_then(|depth| u32::try_from(depth).ok()),
        skip_clone: !clone.enabled,
    })
}

/// The environment's variables in source form, for a path with no vault
/// (server preflight): a `{{ secrets.* }}` value keeps its token, and
/// nothing else is left to resolve because `{{ vars.* }}` is substituted at
/// run creation.
pub fn unresolved_env(settings: &RunEnvironmentSettings) -> BTreeMap<String, String> {
    #[expect(
        clippy::disallowed_methods,
        reason = "preflight has no vault, so an unresolved secret token is carried in source form"
    )]
    settings
        .env
        .iter()
        .map(|(key, value)| (key.clone(), value.as_source()))
        .collect()
}

pub fn local_working_directory_from_environment(
    settings: &RunEnvironmentSettings,
    source_directory: Option<&Path>,
) -> crate::Result<PathBuf> {
    if let Some(cwd) = settings.cwd.as_deref() {
        return Ok(PathBuf::from(cwd));
    }

    let Some(source_directory) = source_directory else {
        return Err(crate::Error::message(
            "local environment requires a server-side working directory; configure `environment.cwd = \"/absolute/path\"` on the selected local environment",
        ));
    };

    if source_directory.is_dir() {
        return Ok(source_directory.to_path_buf());
    }

    Err(crate::Error::message(format!(
        "local environment source_directory does not exist or is not a directory on this server: {}. Configure `environment.cwd = \"/absolute/path\"` on the selected local environment for remote client/server deployments.",
        source_directory.display()
    )))
}

/// The driver spec every provider starts from: the environment's source
/// (an image, a Dockerfile, or a managed directory when it names
/// neither), the run's name and labels, the variables, resources, and
/// network policy. A bundled provider's overlay adjusts what its backend
/// needs.
pub(crate) fn base_spec(options: &SandboxOptions, run_id: Option<&RunId>) -> DriverSpec {
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
    let mut spec = DriverSpec::new(source).network(options.network.clone());
    if let Some(run_id) = run_id {
        spec = spec.name(run_name(run_id));
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
    resources.cpu_cores = options.cpu;
    resources.memory_mb = options.memory_mb();
    resources.disk_mb = options.disk_mb();
    spec.resources(resources)
}

/// The provider-side name of a run's sandbox.
pub(crate) fn run_name(run_id: &RunId) -> String {
    format!("fabro-run-{run_id}")
}

/// The environment's default `allow_all` means "unrestricted", which a
/// provider without network controls already is; asking such a provider
/// for it explicitly would be rejected. An explicit restriction is still
/// requested, and refused by the provider when it cannot honor it.
pub(crate) fn supported_network(
    requested: NetworkPolicy,
    capabilities: &Capabilities,
) -> NetworkPolicy {
    match requested {
        NetworkPolicy::AllowAll if !capabilities.network.allow_all => {
            NetworkPolicy::ProviderDefault
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use fabro_types::SandboxProviderKind;
    use fabro_types::settings::run::{
        EnvironmentImageSettings, EnvironmentLifecycleSettings, EnvironmentNetworkSettings,
        EnvironmentResourcesSettings,
    };
    use fabro_types::settings::{Duration as SettingsDuration, Size};

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
            labels:    HashMap::from([("team".to_string(), "platform".to_string())]),
            env:       HashMap::new(),
        }
    }

    fn run_id() -> RunId {
        "01HY0000000000000000000000".parse().unwrap()
    }

    #[test]
    fn options_without_an_image_ask_for_a_managed_directory() {
        let options = options_from_environment(
            &environment("host"),
            &RunCloneSettings::default(),
            BTreeMap::from([("FOO".to_string(), "bar".to_string())]),
        )
        .unwrap();
        assert!(options.image.is_none());
        assert!(options.dockerfile.is_none());
        assert_eq!(options.clone_depth, Some(100));
        assert!(!options.skip_clone);
        assert!(options.auto_stop.is_none());

        let spec = base_spec(&options, Some(&run_id()));
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
    fn options_with_an_image_map_resources_network_and_lifecycle() {
        let mut settings = environment("e2b");
        settings.image.docker = Some("ubuntu:24.04".to_string());
        settings.resources.cpu = Some(2);
        settings.resources.memory = Some(Size::from_bytes(4_000_000_000));
        settings.network.mode = EnvironmentNetworkMode::Block;
        settings.lifecycle.auto_stop = Some(SettingsDuration::from_std(Duration::from_mins(45)));
        let clone = RunCloneSettings {
            enabled: false,
            depth:   0,
        };
        let options = options_from_environment(&settings, &clone, BTreeMap::new()).unwrap();
        assert_eq!(options.image.as_deref(), Some("ubuntu:24.04"));
        assert!(options.skip_clone);
        assert_eq!(options.clone_depth, None);
        assert_eq!(options.memory_bytes, Some(4_000_000_000));
        assert_eq!(options.memory_mb(), Some(3815));
        assert_eq!(options.auto_stop, Some(Duration::from_mins(45)));

        let spec = base_spec(&options, None);
        assert!(matches!(
            &spec.source,
            SandboxSource::Image { reference } if reference == "ubuntu:24.04"
        ));
        assert_eq!(spec.resources.cpu_cores, Some(2));
        assert_eq!(spec.resources.memory_mb, Some(3815));
        assert!(matches!(spec.network, NetworkPolicy::Block));
        assert!(spec.name.is_none());
        assert!(!spec.labels.contains_key("sh.fabro.run_id"));
    }

    #[test]
    fn an_inline_dockerfile_becomes_the_source_and_a_path_is_rejected() {
        let mut settings = environment("daytona");
        settings.image.dockerfile = Some(DockerfileSource::Inline("FROM ubuntu".to_string()));
        let options =
            options_from_environment(&settings, &RunCloneSettings::default(), BTreeMap::new())
                .unwrap();
        assert_eq!(options.dockerfile.as_deref(), Some("FROM ubuntu"));
        assert!(matches!(
            base_spec(&options, None).source,
            SandboxSource::Dockerfile { content } if content == "FROM ubuntu"
        ));

        settings.image.dockerfile = Some(DockerfileSource::Path {
            path: "Dockerfile".to_string(),
        });
        let error =
            options_from_environment(&settings, &RunCloneSettings::default(), BTreeMap::new())
                .unwrap_err();
        assert!(error.to_string().contains("Dockerfile path"), "{error}");
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
    fn local_working_directory_prefers_environment_cwd() {
        let mut settings = environment("local");
        settings.cwd = Some("/srv/fabro/workspaces/team-a".to_string());
        let missing_source = Path::new("/path/that/should/not/exist");

        let resolved = local_working_directory_from_environment(&settings, Some(missing_source))
            .expect("configured cwd should be accepted");

        assert_eq!(resolved, PathBuf::from("/srv/fabro/workspaces/team-a"));
        assert!(!missing_source.exists());
    }

    #[test]
    fn local_working_directory_uses_existing_source_directory_without_cwd() {
        let settings = environment("local");
        let dir = tempfile::tempdir().unwrap();

        let resolved = local_working_directory_from_environment(&settings, Some(dir.path()))
            .expect("existing source directory should be accepted");

        assert_eq!(resolved, dir.path());
    }

    #[test]
    fn local_working_directory_rejects_missing_source_directory_without_cwd() {
        let settings = environment("local");
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("client-only");

        let err = local_working_directory_from_environment(&settings, Some(&missing))
            .expect_err("missing source directory without cwd should fail");

        let message = err.to_string();
        assert!(
            message.contains("environment.cwd") && message.contains("does not exist"),
            "unexpected error: {message}"
        );
        assert!(!missing.exists());
    }
}
