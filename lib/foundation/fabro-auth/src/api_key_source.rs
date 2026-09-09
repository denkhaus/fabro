//! A credential source holding one operator-supplied API key.
//!
//! Used to validate a pasted key before it is stored: the key is shaped into
//! the provider's declared auth scheme and offered for that provider only.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fabro_vault::Vault;
use lithos_llm::catalog::{Catalog, CatalogProvider, ProviderId};
use lithos_llm::credentials::Credentials;
use tokio::sync::RwLock as AsyncRwLock;

use crate::credential_source::CredentialSource;
use crate::resolve::{ResolveError, credentials_for_api_key};

pub struct ApiKeyCredentialSource {
    provider: ProviderId,
    key:      String,
    vault:    Arc<AsyncRwLock<Vault>>,
}

impl ApiKeyCredentialSource {
    /// A source for `provider` with no vault behind it, so extra headers that
    /// interpolate vault secrets fail to resolve.
    #[must_use]
    pub fn new(provider: ProviderId, key: String) -> Self {
        Self::with_vault(
            provider,
            key,
            Arc::new(AsyncRwLock::new(Vault::from_entries(HashMap::new()))),
        )
    }

    /// A source for `provider` whose extra headers resolve against `vault`.
    #[must_use]
    pub fn with_vault(provider: ProviderId, key: String, vault: Arc<AsyncRwLock<Vault>>) -> Self {
        Self {
            provider,
            key,
            vault,
        }
    }
}

impl std::fmt::Debug for ApiKeyCredentialSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiKeyCredentialSource")
            .field("provider", &self.provider)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl CredentialSource for ApiKeyCredentialSource {
    async fn credentials(&self, provider: &CatalogProvider) -> Result<Credentials, ResolveError> {
        if provider.id() != &self.provider {
            return Err(ResolveError::NotConfigured(provider.id().clone()));
        }
        let vault = self.vault.read().await;
        credentials_for_api_key(provider, self.key.clone(), &vault)
    }

    async fn configured_providers(&self, catalog: &Catalog) -> Vec<ProviderId> {
        catalog
            .provider(self.provider.as_str())
            .ok()
            .map(|provider| vec![provider.id().clone()])
            .unwrap_or_default()
    }
}
