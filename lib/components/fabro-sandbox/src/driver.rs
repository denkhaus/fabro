//! The one place fabro turns provider configuration into a sandbox-driver
//! [`SandboxProvider`].
//!
//! Bundled kinds (`local`, `docker`, `daytona`) link the driver's provider
//! crates in-process. Any other kind launches the configured plugin
//! executable over stdio and supervises it. Callers never learn which they
//! got: both come back as `Arc<dyn SandboxProvider>` tagged with fabro's own
//! [`SandboxProviderKind`], which is what run records and inventory persist.
//!
//! Credentials arrive explicitly. Nothing here reads the process environment:
//! the Daytona key comes from the vault through [`DaytonaCredentials`], and a
//! plugin starts from a scrubbed environment containing only what its
//! settings declare.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use fabro_static::EnvVars;
use fabro_types::settings::server::{
    SandboxPluginSettings, ServerSandboxProviderSettings, ServerSandboxProvidersSettings,
};
use fabro_types::{BundledProvider, SandboxProviderKind};
use sandbox_driver::{
    Capabilities, EventContext, ProviderHealth, ProviderKind, Sandbox, SandboxFilter, SandboxId,
    SandboxProvider, SandboxSpec, SandboxStatus, SnapshotProvider, VolumeProvider,
};
use sandbox_driver_daytona::{DaytonaConfig, DaytonaProvider};
use sandbox_driver_docker::DockerProvider;
use sandbox_driver_host::HostProvider;
use sandbox_driver_protocol::{PluginConfig, PluginSupervisor};

/// Binary naming prefix for plugin discovery: a plugin for kind `e2b` is
/// `fabro-sandbox-e2b` on `PATH` unless the settings name a path.
pub const PLUGIN_BINARY_PREFIX: &str = "fabro-sandbox";

/// `User-Agent` fabro presents to remote sandbox control planes.
pub const USER_AGENT: &str = concat!("fabro-sandbox/", env!("CARGO_PKG_VERSION"));

/// Explicit Daytona credentials. The process environment is never consulted.
#[derive(Clone)]
pub struct DaytonaCredentials {
    pub api_key:         String,
    pub api_url:         Option<String>,
    pub organization_id: Option<String>,
    pub target:          Option<String>,
    /// Shared HTTP client; tests pass a no-proxy client here.
    pub http_client:     Option<reqwest::Client>,
}

impl DaytonaCredentials {
    /// Credentials for a vault API key, with the control-plane URL and
    /// organization taken from `lookup` (server configuration, or the
    /// process environment in a CLI worker). Nothing is read implicitly.
    pub fn from_api_key(api_key: String, lookup: impl Fn(&str) -> Option<String>) -> Self {
        Self {
            api_key,
            api_url: lookup(EnvVars::DAYTONA_API_URL)
                .or_else(|| lookup(EnvVars::DAYTONA_SERVER_URL)),
            organization_id: lookup(EnvVars::DAYTONA_ORGANIZATION_ID),
            target: None,
            http_client: None,
        }
    }
}

impl std::fmt::Debug for DaytonaCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaytonaCredentials")
            .field("api_url", &self.api_url)
            .field("organization_id", &self.organization_id)
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

/// What a process needs to reach every provider a run record can name: the
/// server's provider settings (which kinds are enabled, which run as
/// plugins) and the Daytona credentials from the vault.
#[derive(Clone, Debug, Default)]
pub struct ProviderAccess {
    pub providers: ServerSandboxProvidersSettings,
    pub daytona:   Option<DaytonaCredentials>,
}

impl ProviderAccess {
    /// The settings entry for `kind`. A bundled kind without an entry is
    /// enabled with defaults; any other kind must be configured.
    pub fn settings_for(
        &self,
        kind: &SandboxProviderKind,
    ) -> Option<ServerSandboxProviderSettings> {
        match self.providers.get(kind) {
            Some(settings) => Some(settings.clone()),
            None if kind.bundled().is_some() => Some(ServerSandboxProviderSettings::default()),
            None => None,
        }
    }

    pub fn connect_options(&self) -> ProviderConnectOptions {
        ProviderConnectOptions {
            host_registry_root: None,
            daytona:            self.daytona.clone(),
        }
    }
}

/// Everything besides the settings entry that a provider connection needs.
#[derive(Clone, Debug, Default)]
pub struct ProviderConnectOptions {
    /// Directory where the in-process Host provider records its sandboxes so
    /// they survive a server restart. `None` uses a fresh temporary registry
    /// that is removed when the provider drops.
    pub host_registry_root: Option<PathBuf>,
    /// Required to connect the bundled Daytona provider.
    pub daytona:            Option<DaytonaCredentials>,
}

/// A provider fabro connected, tagged with the kind fabro persists for it.
///
/// The driver's own `provider.kind()` may differ from fabro's kind: fabro's
/// `local` is the driver's `host`. Persist and dispatch on `kind`, never on
/// the driver's name.
#[derive(Clone)]
pub struct ConnectedProvider {
    pub kind:     SandboxProviderKind,
    pub provider: Arc<dyn SandboxProvider>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("sandbox provider `{kind}` is disabled by server.sandbox.providers.{kind}.enabled")]
    Disabled { kind: SandboxProviderKind },
    #[error(
        "sandbox provider `{kind}` has no plugin settings; add server.sandbox.providers.{kind}"
    )]
    MissingPluginSettings { kind: SandboxProviderKind },
    #[error("sandbox provider `daytona` requires DAYTONA_API_KEY in the vault")]
    MissingDaytonaCredentials,
    #[error("sandbox provider `{kind}` is not a valid sandbox-driver kind")]
    InvalidKind {
        kind:   SandboxProviderKind,
        #[source]
        source: sandbox_driver::InvalidIdError,
    },
    #[error("failed to connect sandbox provider `{kind}`")]
    Driver {
        kind:   SandboxProviderKind,
        #[source]
        source: sandbox_driver::Error,
    },
}

/// Connects the provider behind `kind`.
///
/// Bundled kinds return the in-process driver provider. Any other kind
/// launches the plugin named by `settings.plugin` and returns a supervised
/// handle that relaunches it after a crash for new work only. The
/// configured kind is fabro's name for whatever the executable serves; the
/// kind the plugin declares is not compared against it. Disabled entries
/// are refused here so no caller has to remember the policy check.
pub async fn connect_provider(
    kind: &SandboxProviderKind,
    settings: &ServerSandboxProviderSettings,
    options: &ProviderConnectOptions,
) -> Result<ConnectedProvider, ConnectError> {
    if !settings.enabled {
        return Err(ConnectError::Disabled { kind: kind.clone() });
    }
    let driver = |source| ConnectError::Driver {
        kind: kind.clone(),
        source,
    };
    let provider: Arc<dyn SandboxProvider> = match kind.bundled() {
        Some(BundledProvider::Local) => match &options.host_registry_root {
            Some(root) => Arc::new(HostProvider::with_registry(root).await.map_err(driver)?),
            None => Arc::new(HostProvider::new()),
        },
        Some(BundledProvider::Docker) => {
            // The daemon is not required to answer at connect time; `health`
            // reports an unreachable daemon so preflight sees the cause.
            Arc::new(DockerProvider::connect_unverified().map_err(driver)?)
        }
        Some(BundledProvider::Daytona) => {
            let credentials = options
                .daytona
                .as_ref()
                .ok_or(ConnectError::MissingDaytonaCredentials)?;
            let config = DaytonaConfig {
                api_key:         Some(credentials.api_key.clone()),
                jwt_token:       None,
                organization_id: credentials.organization_id.clone(),
                api_url:         credentials.api_url.clone(),
                target:          credentials.target.clone(),
                http_client:     credentials.http_client.clone(),
                user_agent:      Some(USER_AGENT.to_string()),
            };
            Arc::new(
                DaytonaProvider::connect_explicit(config)
                    .await
                    .map_err(driver)?,
            )
        }
        None => {
            let plugin = settings
                .plugin
                .as_ref()
                .ok_or_else(|| ConnectError::MissingPluginSettings { kind: kind.clone() })?;
            Arc::new(PluginBackedProvider::launch(kind, plugin).await?)
        }
    };
    Ok(ConnectedProvider {
        kind: kind.clone(),
        provider,
    })
}

/// A plugin provider that survives its executable crashing.
///
/// Wraps a [`PluginSupervisor`]: every call obtains the current plugin
/// generation, and a closed transport is replaced with a fresh launch before
/// the call. A failed call is never replayed, and handles obtained from an
/// earlier generation stay bound to it; callers rebuild them through
/// [`SandboxProvider::attach`] with the persisted sandbox id.
pub struct PluginBackedProvider {
    kind:         ProviderKind,
    capabilities: Capabilities,
    supervisor:   PluginSupervisor,
}

impl PluginBackedProvider {
    async fn launch(
        kind: &SandboxProviderKind,
        settings: &SandboxPluginSettings,
    ) -> Result<Self, ConnectError> {
        let driver_kind =
            ProviderKind::try_new(kind.as_str()).map_err(|source| ConnectError::InvalidKind {
                kind: kind.clone(),
                source,
            })?;
        let supervisor = PluginSupervisor::new(
            PLUGIN_BINARY_PREFIX,
            plugin_config(driver_kind.clone(), settings),
        );
        // Launch once now so a misconfigured plugin fails at connect time and
        // the declared capabilities are known for preflight.
        let capabilities = supervisor
            .current()
            .await
            .map_err(|source| ConnectError::Driver {
                kind: kind.clone(),
                source,
            })?
            .capabilities()
            .clone();
        Ok(Self {
            kind: driver_kind,
            capabilities,
            supervisor,
        })
    }

    /// Asks the current plugin generation to exit and reaps it.
    pub async fn shutdown(&self) -> sandbox_driver::Result<()> {
        self.supervisor.shutdown().await
    }
}

fn plugin_config(kind: ProviderKind, settings: &SandboxPluginSettings) -> PluginConfig {
    PluginConfig {
        kind,
        path: settings.path.as_deref().map(PathBuf::from),
        sha256: settings.sha256.clone(),
        dev: settings.dev,
        args: settings.args.clone(),
        env: settings
            .env
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<BTreeMap<_, _>>(),
        inherit_env: settings.inherit_env.clone(),
    }
}

#[async_trait]
impl SandboxProvider for PluginBackedProvider {
    fn kind(&self) -> &ProviderKind {
        &self.kind
    }

    fn capabilities(&self) -> &Capabilities {
        &self.capabilities
    }

    async fn create(
        &self,
        spec: &SandboxSpec,
        events: Option<EventContext>,
    ) -> sandbox_driver::Result<Arc<dyn Sandbox>> {
        self.supervisor.current().await?.create(spec, events).await
    }

    async fn attach(
        &self,
        id: &SandboxId,
        events: Option<EventContext>,
    ) -> sandbox_driver::Result<Arc<dyn Sandbox>> {
        self.supervisor.current().await?.attach(id, events).await
    }

    async fn undelete(
        &self,
        id: &SandboxId,
        events: Option<EventContext>,
    ) -> sandbox_driver::Result<Arc<dyn Sandbox>> {
        self.supervisor.current().await?.undelete(id, events).await
    }

    async fn delete(
        &self,
        id: &SandboxId,
        events: Option<EventContext>,
    ) -> sandbox_driver::Result<()> {
        self.supervisor.current().await?.delete(id, events).await
    }

    async fn list(&self, filter: &SandboxFilter) -> sandbox_driver::Result<Vec<SandboxStatus>> {
        self.supervisor.current().await?.list(filter).await
    }

    async fn health(&self) -> sandbox_driver::Result<ProviderHealth> {
        self.supervisor.current().await?.health().await
    }

    /// Snapshot and volume management cross the wire per plugin generation,
    /// which these borrowing accessors cannot express. Fabro drives
    /// snapshots on the bundled Daytona provider only, so a plugin reports
    /// none until a generation-aware accessor exists.
    fn snapshots(&self) -> Option<&dyn SnapshotProvider> {
        None
    }

    fn volumes(&self) -> Option<&dyn VolumeProvider> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(plugin: Option<SandboxPluginSettings>) -> ServerSandboxProviderSettings {
        ServerSandboxProviderSettings {
            enabled: true,
            plugin,
        }
    }

    #[tokio::test]
    async fn disabled_entries_are_refused_before_any_connection() {
        let error = connect_provider(
            &SandboxProviderKind::DOCKER,
            &ServerSandboxProviderSettings {
                enabled: false,
                plugin:  None,
            },
            &ProviderConnectOptions::default(),
        )
        .await
        .err()
        .expect("disabled provider must not connect");
        assert!(
            matches!(error, ConnectError::Disabled { kind } if kind == SandboxProviderKind::DOCKER)
        );
    }

    #[tokio::test]
    async fn daytona_requires_explicit_credentials() {
        let error = connect_provider(
            &SandboxProviderKind::DAYTONA,
            &settings(None),
            &ProviderConnectOptions::default(),
        )
        .await
        .err()
        .expect("daytona must not fall back to the environment");
        assert!(matches!(error, ConnectError::MissingDaytonaCredentials));
    }

    #[tokio::test]
    async fn plugin_kinds_require_plugin_settings() {
        let kind = SandboxProviderKind::try_new("e2b").unwrap();
        let error = connect_provider(&kind, &settings(None), &ProviderConnectOptions::default())
            .await
            .err()
            .expect("a plugin kind without settings cannot launch");
        assert!(matches!(error, ConnectError::MissingPluginSettings { kind: k } if k == kind));
    }

    #[tokio::test]
    async fn local_connects_the_host_provider_in_process() {
        let registry = tempfile::tempdir().unwrap();
        let connected = connect_provider(
            &SandboxProviderKind::LOCAL,
            &settings(None),
            &ProviderConnectOptions {
                host_registry_root: Some(registry.path().to_path_buf()),
                daytona:            None,
            },
        )
        .await
        .expect("host provider connects without external services");
        assert_eq!(connected.kind, SandboxProviderKind::LOCAL);
        assert_eq!(connected.provider.kind().as_str(), "host");
    }

    #[test]
    fn plugin_config_carries_every_launch_setting() {
        let config = plugin_config(
            ProviderKind::try_new("e2b").unwrap(),
            &SandboxPluginSettings {
                path:        Some("/opt/e2b".to_string()),
                sha256:      Some("abc".to_string()),
                dev:         true,
                args:        vec!["--flag".to_string()],
                env:         BTreeMap::from([("A".to_string(), "1".to_string())]),
                inherit_env: vec!["PATH".to_string()],
            },
        );
        assert_eq!(
            config.path.as_deref(),
            Some(std::path::Path::new("/opt/e2b"))
        );
        assert_eq!(config.sha256.as_deref(), Some("abc"));
        assert!(config.dev);
        assert_eq!(config.args, vec!["--flag"]);
        assert_eq!(config.env.get("A").map(String::as_str), Some("1"));
        assert_eq!(config.inherit_env, vec!["PATH"]);
    }
}
