use std::sync::Arc;

use async_trait::async_trait;
use fabro_vault::Vault;
use lithos_llm::catalog::{Catalog, CatalogProvider, ProviderId};
use lithos_llm::credentials::Credentials;
use tokio::sync::RwLock as AsyncRwLock;

use crate::credential_source::CredentialSource;
use crate::{CredentialResolver, EnvLookup, ResolveError};

/// Credentials backed by an in-memory [`Vault`] plus an environment lookup.
#[derive(Clone)]
pub struct VaultCredentialSource {
    vault:    Arc<AsyncRwLock<Vault>>,
    resolver: CredentialResolver,
}

impl VaultCredentialSource {
    #[must_use]
    pub fn new(vault: Arc<AsyncRwLock<Vault>>) -> Self {
        let resolver = CredentialResolver::new(Arc::clone(&vault));
        Self { vault, resolver }
    }

    #[must_use]
    pub fn with_env_lookup<F>(vault: Arc<AsyncRwLock<Vault>>, env_lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String> + Send + Sync + 'static,
    {
        let env_lookup: EnvLookup = Arc::new(env_lookup);
        let resolver = CredentialResolver::with_env_lookup(Arc::clone(&vault), env_lookup);
        Self { vault, resolver }
    }

    #[must_use]
    pub fn vault_only(vault: Arc<AsyncRwLock<Vault>>) -> Self {
        Self::with_env_lookup(vault, |_| None)
    }

    pub(crate) async fn snapshot(&self) -> Vault {
        self.vault.read().await.clone()
    }
}

impl std::fmt::Debug for VaultCredentialSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultCredentialSource")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl CredentialSource for VaultCredentialSource {
    async fn credentials(&self, provider: &CatalogProvider) -> Result<Credentials, ResolveError> {
        self.resolver.resolve(provider).await
    }

    async fn configured_providers(&self, catalog: &Catalog) -> Vec<ProviderId> {
        let vault = self.vault.read().await;
        self.resolver.configured_providers(&vault, catalog)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use chrono::{Duration, Utc};
    use fabro_vault::Vault;
    use lithos_llm::catalog::ProviderId;
    use tokio::sync::RwLock as AsyncRwLock;

    use super::VaultCredentialSource;
    use crate::credential::{OAuthConfig, OAuthCredential, OAuthTokens};
    use crate::test_support::test_catalog;
    use crate::vault_ext::{vault_set_oauth, vault_set_token};
    use crate::{CredentialSource, ResolveError};

    fn expired_openai_credential() -> OAuthCredential {
        OAuthCredential {
            tokens:     OAuthTokens {
                access_token:  "expired-access".to_string(),
                refresh_token: Some("refresh-token".to_string()),
                expires_at:    Utc::now() - Duration::hours(1),
            },
            config:     OAuthConfig {
                auth_url:     "https://auth.openai.com".to_string(),
                token_url:    "http://127.0.0.1:9/oauth/token".to_string(),
                client_id:    "client".to_string(),
                scopes:       vec!["openid".to_string()],
                redirect_uri: Some("https://example.com/callback".to_string()),
                use_pkce:     true,
            },
            account_id: Some("acct_123".to_string()),
        }
    }

    #[tokio::test]
    async fn resolve_all_separates_ready_providers_from_auth_issues() {
        let mut vault = Vault::from_entries(HashMap::new());
        vault_set_oauth(
            &mut vault,
            crate::OPENAI_CODEX_VAULT_SECRET_NAME,
            &expired_openai_credential(),
        )
        .unwrap();
        vault_set_token(&mut vault, "ANTHROPIC_API_KEY", "anthropic-key").unwrap();

        let source =
            VaultCredentialSource::with_env_lookup(Arc::new(AsyncRwLock::new(vault)), |_| None);
        let catalog = test_catalog();

        let resolved = source.resolve_all(&catalog).await;

        assert_eq!(resolved.ready, vec![ProviderId::new("anthropic")]);
        assert_eq!(resolved.auth_issues.len(), 1);
        assert!(matches!(
            &resolved.auth_issues[0].1,
            ResolveError::RefreshFailed { provider, .. } if provider.as_str() == "openai-codex"
        ));
    }

    #[tokio::test]
    async fn configured_providers_reads_from_vault_without_refreshing() {
        let mut vault = Vault::from_entries(HashMap::new());
        vault_set_token(&mut vault, "OPENAI_API_KEY", "openai-key").unwrap();
        vault_set_token(&mut vault, "ANTHROPIC_API_KEY", "anthropic-key").unwrap();
        let source =
            VaultCredentialSource::with_env_lookup(Arc::new(AsyncRwLock::new(vault)), |_| None);
        let catalog = test_catalog();

        assert_eq!(source.configured_providers(&catalog).await, vec![
            ProviderId::new("anthropic"),
            ProviderId::new("openai")
        ]);
    }

    #[tokio::test]
    async fn vault_only_ignores_env_lookup_values() {
        let catalog = test_catalog();
        let env_backed = VaultCredentialSource::with_env_lookup(
            Arc::new(AsyncRwLock::new(Vault::from_entries(HashMap::new()))),
            |name| (name == "OPENAI_API_KEY").then(|| "env-openai-key".to_string()),
        );
        assert_eq!(env_backed.configured_providers(&catalog).await, vec![
            ProviderId::new("openai")
        ]);

        let vault_only = VaultCredentialSource::vault_only(Arc::new(AsyncRwLock::new(
            Vault::from_entries(HashMap::new()),
        )));
        assert!(vault_only.configured_providers(&catalog).await.is_empty());
        let resolved = vault_only.resolve_all(&catalog).await;
        assert!(resolved.ready.is_empty());
        assert!(resolved.auth_issues.is_empty());
    }
}
