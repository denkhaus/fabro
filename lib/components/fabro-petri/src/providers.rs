//! Built-in sandbox providers shared by Petri execution and server access.
//!
//! Configuration is captured by the caller, with Daytona credentials from
//! the vault. Factories connect lazily per run; a Host-only run needs neither
//! Docker nor Daytona. Host registry ownership stays with Petri: server
//! attach uses an observer instead of these factories.

use std::sync::Arc;

use async_trait::async_trait;
use fabro_static::EnvVars;
use petri_runtime::{
    InProcessProviders, ProviderContext, ProviderFactory, ProviderNetwork, fingerprint,
};
use sandbox_driver::{AuthError, Error, ProviderKind, SandboxProvider};
use sandbox_driver_daytona::{DaytonaConfig, DaytonaProvider};
use sandbox_driver_docker::DockerProvider;
use sandbox_driver_host::HostProvider;

const USER_AGENT: &str = concat!("fabro-server/", env!("CARGO_PKG_VERSION"));

/// Explicit Daytona credentials: the SDK's configuration with the API key
/// always present and a `Debug` that never prints it. The process
/// environment is never consulted.
#[derive(Clone)]
pub struct DaytonaCredentials(DaytonaConfig);

impl DaytonaCredentials {
    /// Credentials for `api_key` against Daytona's public control plane,
    /// presenting Fabro's `User-Agent`.
    #[must_use]
    pub fn new(api_key: String) -> Self {
        Self(DaytonaConfig {
            api_key: Some(api_key),
            user_agent: Some(USER_AGENT.to_string()),
            ..DaytonaConfig::default()
        })
    }

    /// Credentials for a vault API key, with the control-plane URL and
    /// organization taken from `lookup` (server configuration). Nothing is
    /// read implicitly.
    pub fn from_api_key(api_key: String, lookup: impl Fn(&str) -> Option<String>) -> Self {
        Self::new(api_key)
            .with_api_url(
                lookup(EnvVars::DAYTONA_API_URL).or_else(|| lookup(EnvVars::DAYTONA_SERVER_URL)),
            )
            .with_organization_id(lookup(EnvVars::DAYTONA_ORGANIZATION_ID))
            .with_target(lookup(EnvVars::DAYTONA_TARGET))
    }

    /// The control-plane URL; Daytona's public API when `None`.
    #[must_use]
    pub fn with_api_url(mut self, api_url: Option<String>) -> Self {
        self.0.api_url = api_url;
        self
    }

    #[must_use]
    pub fn with_organization_id(mut self, organization_id: Option<String>) -> Self {
        self.0.organization_id = organization_id;
        self
    }

    /// The configured Daytona placement target, kept unset when omitted.
    #[must_use]
    pub fn with_target(mut self, target: Option<String>) -> Self {
        self.0.target = target;
        self
    }

    /// A shared HTTP client; tests pass a no-proxy client here.
    #[must_use]
    pub fn with_http_client(mut self, http_client: Option<fabro_http::HttpClient>) -> Self {
        self.0.http_client = http_client;
        self
    }

    /// The SDK configuration the driver's Daytona provider connects with.
    #[must_use]
    fn config(&self) -> &DaytonaConfig {
        &self.0
    }
}

impl std::fmt::Debug for DaytonaCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DaytonaCredentials")
            .field("api_url", &self.0.api_url)
            .field("organization_id", &self.0.organization_id)
            .field("target", &self.0.target)
            .finish_non_exhaustive()
    }
}

/// The provider configuration supplied to a run or prune. Defaults are
/// local Docker selection and no Daytona credentials; constructing this
/// configuration never connects to a backend or requires a credential.
#[derive(Clone, Debug, Default)]
pub struct SandboxProviderConfig {
    pub docker_host:         Option<String>,
    pub docker_host_address: Option<String>,
    pub daytona:             Option<DaytonaCredentials>,
}

impl SandboxProviderConfig {
    /// Snapshot the Docker network/fingerprint selection from the same
    /// environment the Docker client uses. Daytona credentials are explicit.
    pub fn from_lookup(
        daytona: Option<DaytonaCredentials>,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Self {
        Self {
            docker_host: lookup(EnvVars::DOCKER_HOST),
            docker_host_address: lookup(EnvVars::PETRI_SANDBOX_DOCKER_HOST_ADDRESS)
                .filter(|value| !value.trim().is_empty()),
            daytona,
        }
    }
}

/// The Docker connection used by both the server and Petri. The driver reads
/// the caller process's Docker endpoint/TLS environment; health is checked
/// by the caller so diagnostics can report an unavailable daemon.
#[expect(
    clippy::result_large_err,
    reason = "preserve sandbox-driver's typed error, as required by Petri's ProviderFactory contract"
)]
pub fn connect_docker() -> sandbox_driver::Result<Arc<dyn SandboxProvider>> {
    Ok(Arc::new(DockerProvider::connect_unverified()?))
}

/// Connect using only the vault credentials and explicit control-plane
/// configuration; no ambient secret fallback is permitted.
pub async fn connect_daytona(
    credentials: &DaytonaCredentials,
) -> sandbox_driver::Result<Arc<dyn SandboxProvider>> {
    Ok(Arc::new(
        DaytonaProvider::connect_explicit(credentials.config().clone()).await?,
    ))
}

/// One lazy factory per built-in kind. Missing Daytona credentials fail
/// only when a Daytona scope is acquired, never for admission or a Host run.
pub fn built_in_providers(config: &SandboxProviderConfig) -> InProcessProviders {
    InProcessProviders::new()
        .with(Arc::new(HostFactory))
        .with(Arc::new(DockerFactory {
            host:         config.docker_host.clone(),
            host_address: config.docker_host_address.clone(),
        }))
        .with(Arc::new(DaytonaFactory(config.daytona.clone())))
}

struct HostFactory;

#[async_trait]
impl ProviderFactory for HostFactory {
    fn kind(&self) -> &'static str {
        "host"
    }

    fn fingerprint_seed(&self, context: &ProviderContext) -> String {
        fingerprint::host(
            context
                .host_registry()
                .expect("Petri supplies the Host registry"),
        )
    }

    fn network(&self) -> ProviderNetwork {
        ProviderNetwork::host()
    }

    async fn connect(
        &self,
        context: &ProviderContext,
    ) -> sandbox_driver::Result<Arc<dyn SandboxProvider>> {
        let registry = context
            .host_registry()
            .expect("Petri supplies the Host registry");
        Ok(Arc::new(HostProvider::with_registry(registry).await?))
    }
}

struct DockerFactory {
    host:         Option<String>,
    host_address: Option<String>,
}

impl DockerFactory {
    fn seed(&self) -> String {
        fingerprint::docker(self.host.as_deref())
    }
}

#[async_trait]
impl ProviderFactory for DockerFactory {
    fn kind(&self) -> &'static str {
        "docker"
    }

    fn fingerprint_seed(&self, _context: &ProviderContext) -> String {
        self.seed()
    }

    fn network(&self) -> ProviderNetwork {
        ProviderNetwork::docker(self.host.as_deref(), self.host_address.as_deref())
    }

    async fn connect(
        &self,
        _context: &ProviderContext,
    ) -> sandbox_driver::Result<Arc<dyn SandboxProvider>> {
        connect_docker()
    }
}

struct DaytonaFactory(Option<DaytonaCredentials>);

impl DaytonaFactory {
    fn seed(&self) -> String {
        let config = self.0.as_ref().map(DaytonaCredentials::config);
        fingerprint::daytona(
            config.and_then(|config| config.api_url.as_deref()),
            config.and_then(|config| config.organization_id.as_deref()),
            config.and_then(|config| config.target.as_deref()),
        )
    }
}

#[async_trait]
impl ProviderFactory for DaytonaFactory {
    fn kind(&self) -> &'static str {
        "daytona"
    }

    fn fingerprint_seed(&self, _context: &ProviderContext) -> String {
        self.seed()
    }

    fn region(&self) -> Option<&str> {
        self.0
            .as_ref()
            .and_then(|credentials| credentials.config().target.as_deref())
            .filter(|region| !region.is_empty())
    }

    fn network(&self) -> ProviderNetwork {
        ProviderNetwork::none()
    }

    async fn connect(
        &self,
        _context: &ProviderContext,
    ) -> sandbox_driver::Result<Arc<dyn SandboxProvider>> {
        let credentials = self.0.as_ref().ok_or_else(|| Error::Auth(AuthError::new(
            ProviderKind::try_new("daytona").expect("the built-in provider kind is valid"),
            "Daytona requires DAYTONA_API_KEY in the vault; run `fabro secret set DAYTONA_API_KEY`",
        )))?;
        connect_daytona(credentials).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_fingerprint_keeps_the_plugin_namespace_for_unset_and_configured_hosts() {
        for (host, expected) in [
            (None, "docker:default"),
            (Some("  "), "docker:default"),
            (Some(" tcp://daemon:2376 "), "docker:tcp://daemon:2376"),
            (
                Some("unix:///var/run/docker.sock"),
                "docker:unix:///var/run/docker.sock",
            ),
        ] {
            let config = SandboxProviderConfig::from_lookup(None, |name| {
                (name == EnvVars::DOCKER_HOST)
                    .then(|| host.map(str::to_owned))
                    .flatten()
            });
            let factory = DockerFactory {
                host:         config.docker_host,
                host_address: config.docker_host_address,
            };
            assert_eq!(factory.seed(), expected);
        }
    }

    #[test]
    fn daytona_fingerprint_keeps_unset_values_and_the_configured_target() {
        let unset = DaytonaCredentials::from_api_key("test-key".to_string(), |_| None);
        assert_eq!(DaytonaFactory(Some(unset)).seed(), "daytona:::");
        let configured =
            DaytonaCredentials::from_api_key("test-key".to_string(), |name| match name {
                EnvVars::DAYTONA_API_URL => Some("https://daytona.example".to_string()),
                EnvVars::DAYTONA_SERVER_URL => Some("https://ignored.example".to_string()),
                EnvVars::DAYTONA_ORGANIZATION_ID => Some("org-1".to_string()),
                EnvVars::DAYTONA_TARGET => Some("us".to_string()),
                _ => None,
            });
        let factory = DaytonaFactory(Some(configured));
        assert_eq!(factory.seed(), "daytona:https://daytona.example:org-1:us");
        assert_eq!(factory.region(), Some("us"));
        let blank =
            DaytonaCredentials::new("test-key".to_string()).with_target(Some(String::new()));
        assert_eq!(DaytonaFactory(Some(blank)).region(), None);
    }

    #[test]
    fn daytona_url_alias_and_http_client_survive_the_shared_configuration() {
        let credentials = DaytonaCredentials::from_api_key("test-key".to_string(), |name| {
            (name == EnvVars::DAYTONA_SERVER_URL).then(|| "https://alias.example".to_string())
        })
        .with_http_client(Some(fabro_test::test_http_client()));
        assert_eq!(
            credentials.config().api_url.as_deref(),
            Some("https://alias.example")
        );
        assert!(credentials.config().http_client.is_some());
    }

    #[test]
    fn provider_configuration_debug_never_prints_the_key() {
        let key = "dtn_test_sensitive_value";
        let config = SandboxProviderConfig::from_lookup(
            Some(DaytonaCredentials::from_api_key(key.to_string(), |_| None)),
            |_| None,
        );
        let rendered = format!("{config:?}");
        assert!(!rendered.contains(key));
    }
}
