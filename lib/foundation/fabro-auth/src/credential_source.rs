//! Per-attempt credential lookup for LLM providers.
//!
//! [`CredentialSource`] is Fabro's storage-aware credential seam: the vault,
//! the SQL secret store, and the process environment each implement it.
//! [`lithos_credentials`] adapts a source into the lithos
//! [`CredentialProvider`] the client calls before every provider attempt, so a
//! refreshed OAuth token is picked up by the next retry.

use std::sync::Arc;

use async_trait::async_trait;
use lithos_llm::catalog::{Catalog, CatalogProvider, ProviderId};
use lithos_llm::credentials::{CredentialError, CredentialProvider, Credentials};

use crate::{ResolveError, auth_issue_message};

/// Which providers a source can serve right now, and why the rest cannot.
#[derive(Debug, Default)]
pub struct ResolvedCredentials {
    /// Enabled providers whose credentials resolved.
    pub ready:       Vec<ProviderId>,
    /// Enabled providers with credential material that failed to resolve,
    /// such as an expired OAuth token that could not be refreshed. Providers
    /// with no material at all are not issues; they are simply absent.
    pub auth_issues: Vec<(ProviderId, ResolveError)>,
}

impl ResolvedCredentials {
    /// A human-readable line per auth issue.
    #[must_use]
    pub fn issue_messages(&self) -> Vec<String> {
        self.auth_issues
            .iter()
            .map(|(provider, issue)| auth_issue_message(provider, issue))
            .collect()
    }
}

#[async_trait]
pub trait CredentialSource: Send + Sync {
    /// Resolves `provider`'s credentials for one request attempt.
    async fn credentials(&self, provider: &CatalogProvider) -> Result<Credentials, ResolveError>;

    /// Providers with credential material present. Does not refresh or
    /// validate anything, so it is cheap enough for listings.
    async fn configured_providers(&self, catalog: &Catalog) -> Vec<ProviderId>;

    /// Resolves every enabled provider once, separating the ready set from
    /// the providers that have material but cannot use it.
    async fn resolve_all(&self, catalog: &Catalog) -> ResolvedCredentials {
        let mut resolved = ResolvedCredentials::default();
        for provider in catalog.providers().filter(|provider| provider.is_enabled()) {
            match self.credentials(provider).await {
                Ok(_) => resolved.ready.push(provider.id().clone()),
                Err(ResolveError::NotConfigured(_)) => {}
                Err(err) => resolved.auth_issues.push((provider.id().clone(), err)),
            }
        }
        resolved
    }
}

/// Adapts a [`CredentialSource`] into the lithos credential provider.
#[must_use]
pub fn lithos_credentials(source: Arc<dyn CredentialSource>) -> Arc<dyn CredentialProvider> {
    Arc::new(SourceCredentialProvider { source })
}

struct SourceCredentialProvider {
    source: Arc<dyn CredentialSource>,
}

#[async_trait]
impl CredentialProvider for SourceCredentialProvider {
    async fn credentials(
        &self,
        provider: &CatalogProvider,
    ) -> Result<Credentials, CredentialError> {
        self.source.credentials(provider).await.map_err(|err| {
            tracing::warn!(
                provider = %provider.id(),
                error = %err,
                "LLM credentials could not be resolved for this attempt"
            );
            match err {
                ResolveError::NotConfigured(provider) => {
                    CredentialError::NotConfigured { provider }
                }
                ResolveError::SchemeMismatch { provider, .. } => {
                    CredentialError::SchemeMismatch { provider }
                }
                other => CredentialError::NotConfigured {
                    provider: other.provider().clone(),
                },
            }
        })
    }
}
