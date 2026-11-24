//! The admission crate's view of the caller's world.
//!
//! Admission never sees server state: the app derives these plain inputs and
//! wires the two callbacks ([`GithubCredentialsSource`],
//! [`ProviderHealthSource`]) that stand in for the app-owned secret store and
//! sandbox-access machinery (work#1672).

use std::sync::Arc;

use fabro_llm::lithos_catalog::Catalog;
use fabro_types::SandboxProviderKind;
use fabro_types::settings::server::GithubIntegrationSettings;
use futures_util::future::BoxFuture;
use sandbox_driver::ProviderHealth;

/// Resolves GitHub credentials for an integration's strategy. The app owns
/// the vault; admission only asks for the credential it may present.
pub type GithubCredentialsSource = Arc<
    dyn Fn(
            &GithubIntegrationSettings,
        ) -> BoxFuture<'static, anyhow::Result<Option<fabro_github::GitHubCredentials>>>
        + Send
        + Sync,
>;

/// Runs the sandbox provider's own health check for a kind. Encapsulates the
/// app's provider access (credentials, connections): admission reports the
/// outcome, it never holds the access bundle.
pub type ProviderHealthSource = Arc<
    dyn Fn(&SandboxProviderKind) -> BoxFuture<'static, anyhow::Result<ProviderHealth>>
        + Send
        + Sync,
>;

/// The plain inputs the admission cluster reads from its caller.
#[derive(Clone)]
pub struct AdmissionContext {
    /// The model catalog: the preflight's model/provider checks read it.
    pub catalog:            Arc<Catalog>,
    /// The server settings snapshot: sandbox provider policy and the GitHub
    /// integration settings the preflight checks read.
    pub server_settings:    Arc<fabro_types::ServerSettings>,
    /// GitHub credential resolution (see [`GithubCredentialsSource`]).
    pub github_credentials: GithubCredentialsSource,
    /// Sandbox provider health checks (see [`ProviderHealthSource`]).
    pub provider_health:    ProviderHealthSource,
}
