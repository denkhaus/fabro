use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fabro_vault::Vault;
use lithos_llm::catalog::{Catalog, CatalogProvider, ProviderId};
use lithos_llm::credentials::Credentials;
use tokio::sync::RwLock as AsyncRwLock;

use crate::{CredentialSource, EnvLookup, ResolveError, VaultCredentialSource};

/// A credential source for provider credentials declared as `env:<NAME>`.
///
/// This public SDK facade does not resolve `{{ secrets.NAME }}` header
/// interpolation, so providers whose headers come from the vault stay
/// unconfigured here.
#[derive(Clone)]
pub struct EnvCredentialSource {
    inner: VaultCredentialSource,
}

impl EnvCredentialSource {
    #[must_use]
    #[expect(
        clippy::disallowed_methods,
        reason = "EnvCredentialSource is the provider credential process-env facade."
    )]
    pub fn new() -> Self {
        Self::with_env_lookup(Arc::new(|name| std::env::var(name).ok()))
    }

    #[must_use]
    pub fn with_env_lookup(env_lookup: EnvLookup) -> Self {
        let vault = Arc::new(AsyncRwLock::new(Vault::from_entries(HashMap::new())));
        let inner = VaultCredentialSource::with_env_lookup(vault, move |name| env_lookup(name));
        Self { inner }
    }
}

impl std::fmt::Debug for EnvCredentialSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvCredentialSource")
            .finish_non_exhaustive()
    }
}

impl Default for EnvCredentialSource {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CredentialSource for EnvCredentialSource {
    async fn credentials(&self, provider: &CatalogProvider) -> Result<Credentials, ResolveError> {
        self.inner.credentials(provider).await
    }

    async fn configured_providers(&self, catalog: &Catalog) -> Vec<ProviderId> {
        self.inner.configured_providers(catalog).await
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use lithos_llm::catalog::ProviderId;

    use super::EnvCredentialSource;
    use crate::CredentialSource;
    use crate::test_support::test_catalog;

    fn test_source(entries: &[(&str, &str)]) -> EnvCredentialSource {
        let entries: HashMap<String, String> = entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        EnvCredentialSource::with_env_lookup(Arc::new(move |name| entries.get(name).cloned()))
    }

    #[tokio::test]
    async fn configured_providers_reads_injected_provider_env() {
        let source = test_source(&[("ANTHROPIC_API_KEY", "anthropic-key")]);
        assert_eq!(source.configured_providers(&test_catalog()).await, vec![
            ProviderId::new("anthropic")
        ]);
    }

    #[tokio::test]
    async fn modal_env_vars_do_not_replace_vault_secrets() {
        let source = test_source(&[
            ("MODAL_TOKEN_ID", "wk-test"),
            ("MODAL_TOKEN_SECRET", "ws-test"),
        ]);
        let catalog = test_catalog();
        let modal = ProviderId::new("modal");
        assert!(!source.configured_providers(&catalog).await.contains(&modal));
        let err = source
            .credentials(catalog.provider("modal").unwrap())
            .await
            .unwrap_err();
        assert!(matches!(err, crate::ResolveError::Interpolation { .. }));
    }
}
