//! A credential source holding one operator-supplied API key.
//!
//! Used to validate a pasted key before it is stored: the key stands in for
//! the first secret the provider conventionally reads, so lithos shapes it
//! into the provider's auth scheme exactly as a stored secret would be.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fabro_vault::Vault;
use lithos_llm::catalog::{Catalog, CatalogProvider, ProviderId};
use lithos_llm::credentials::{ConventionalCredentials, CredentialProvider, Credentials};
use tokio::sync::RwLock as AsyncRwLock;

use crate::credential_source::CredentialSource;
use crate::error::ResolveError;
use crate::secrets::expected_secret_name;
use crate::vault_source::{auth_scheme_name, interpolated_headers, resolve_error};

pub struct ApiKeyCredentialSource {
    provider: ProviderId,
    key:      String,
    vault:    Arc<AsyncRwLock<Vault>>,
}

impl ApiKeyCredentialSource {
    /// A source for `provider` with no vault behind it, so header secrets the
    /// provider interpolates from the vault fail to resolve.
    #[must_use]
    pub fn new(provider: ProviderId, key: String) -> Self {
        Self::with_vault(
            provider,
            key,
            Arc::new(AsyncRwLock::new(Vault::from_entries(HashMap::new()))),
        )
    }

    /// A source for `provider` whose interpolated headers resolve against
    /// `vault`.
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

/// Shapes a caller-supplied API key into the provider's credentials.
pub(crate) async fn credentials_for_api_key(
    provider: &CatalogProvider,
    key: String,
    vault: &Vault,
) -> Result<Credentials, ResolveError> {
    let Some(name) = expected_secret_name(provider) else {
        return Err(ResolveError::SchemeMismatch {
            provider: provider.id().clone(),
            scheme:   auth_scheme_name(provider.auth()).to_string(),
        });
    };
    let interpolated = interpolated_headers(vault, provider)?;
    let mut credentials = ConventionalCredentials::new()
        .with_lookup(move |candidate| (candidate == name).then(|| key.clone()))
        .credentials(provider)
        .await
        .map_err(|err| resolve_error(provider, &err))?;
    if let Credentials::Http(http) = &mut credentials {
        http.extra_headers.extend(interpolated);
    }
    Ok(credentials)
}

#[async_trait]
impl CredentialSource for ApiKeyCredentialSource {
    async fn credentials(&self, provider: &CatalogProvider) -> Result<Credentials, ResolveError> {
        if provider.id() != &self.provider {
            return Err(ResolveError::NotConfigured(provider.id().clone()));
        }
        let vault = self.vault.read().await.clone();
        credentials_for_api_key(provider, self.key.clone(), &vault).await
    }

    async fn configured_providers(&self, catalog: &Catalog) -> Vec<ProviderId> {
        catalog
            .provider(self.provider.as_str())
            .ok()
            .map(|provider| vec![provider.id().clone()])
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use lithos_llm::credentials::{HttpAuthentication, HttpCredentials};

    use super::*;
    use crate::secrets::accepts_api_key;
    use crate::test_support::test_catalog;

    #[tokio::test]
    async fn api_key_credentials_follow_the_provider_scheme() {
        let catalog = test_catalog();
        let vault = Vault::from_entries(HashMap::new());
        let openai = credentials_for_api_key(
            catalog.provider("openai").unwrap(),
            "sk-test".to_string(),
            &vault,
        )
        .await
        .unwrap();
        assert!(matches!(
            openai,
            Credentials::Http(HttpCredentials {
                auth: HttpAuthentication::Bearer(secret),
                ..
            }) if secret.expose_secret() == "sk-test"
        ));
        let bedrock = credentials_for_api_key(
            catalog.provider("bedrock").unwrap(),
            "sk-test".to_string(),
            &vault,
        )
        .await
        .unwrap();
        assert!(matches!(bedrock, Credentials::BedrockBearer(_)));
        let modal = credentials_for_api_key(
            catalog.provider("modal").unwrap(),
            "sk-test".to_string(),
            &vault,
        )
        .await;
        assert!(modal.is_err(), "modal has no single-key scheme");
        assert!(!accepts_api_key(catalog.provider("modal").unwrap()));
        assert!(accepts_api_key(catalog.provider("openai").unwrap()));
        assert!(accepts_api_key(catalog.provider("bedrock").unwrap()));
        assert!(!accepts_api_key(catalog.provider("ollama").unwrap()));
    }
}
