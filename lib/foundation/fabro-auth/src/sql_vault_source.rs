use std::sync::Arc;

use async_trait::async_trait;
use fabro_types::SecretType;
use fabro_vault::{SecretSnapshot, SecretStore, SecretStoreError, Vault};
use lithos_llm::catalog::{Catalog, CatalogProvider, ProviderId};
use lithos_llm::credentials::Credentials;
use tokio::sync::RwLock;
use tracing::error;

use crate::credential_source::CredentialSource;
use crate::{EnvLookup, ResolveError, VaultCredentialSource};

/// Credentials backed by the SQL secret store.
///
/// Every lookup snapshots the store, resolves against the snapshot, and
/// writes refreshed OAuth tokens back with a revision check so two concurrent
/// refreshes cannot clobber each other.
#[derive(Clone)]
pub struct SqlVaultCredentialSource {
    store:      Arc<SecretStore>,
    env_lookup: EnvLookup,
}

impl SqlVaultCredentialSource {
    #[must_use]
    #[expect(
        clippy::disallowed_methods,
        reason = "SqlVaultCredentialSource::new owns the process-env fallback used after vault \
                  lookup."
    )]
    pub fn new(store: Arc<SecretStore>) -> Self {
        Self::with_env_lookup(store, |name| std::env::var(name).ok())
    }

    #[must_use]
    pub fn vault_only(store: Arc<SecretStore>) -> Self {
        Self::with_env_lookup(store, |_| None)
    }

    #[must_use]
    pub fn with_env_lookup<F>(store: Arc<SecretStore>, env_lookup: F) -> Self
    where
        F: Fn(&str) -> Option<String> + Send + Sync + 'static,
    {
        Self {
            store,
            env_lookup: Arc::new(env_lookup),
        }
    }

    fn source_for_snapshot(&self, snapshot: SecretSnapshot) -> VaultCredentialSource {
        let env_lookup = Arc::clone(&self.env_lookup);
        VaultCredentialSource::with_env_lookup(
            Arc::new(RwLock::new(snapshot.into_vault())),
            move |name| env_lookup(name),
        )
    }

    async fn persist_oauth_refreshes(
        &self,
        before: &Vault,
        after: &Vault,
    ) -> Result<bool, SecretStoreError> {
        for (name, after_entry) in after.entries() {
            if after_entry.secret_type != SecretType::Oauth {
                continue;
            }
            let Some(before_entry) = before.get_entry(name) else {
                continue;
            };
            if before_entry.value == after_entry.value {
                continue;
            }
            match self
                .store
                .replace_if_revision(
                    name,
                    before_entry.revision,
                    &after_entry.value,
                    SecretType::Oauth,
                )
                .await
            {
                Ok(_) => {}
                Err(SecretStoreError::StaleRevision { .. }) => return Ok(false),
                Err(err) => return Err(err),
            }
        }
        Ok(true)
    }

    fn store_error(provider: &ProviderId, err: SecretStoreError) -> ResolveError {
        ResolveError::RefreshFailed {
            provider: provider.clone(),
            source:   anyhow::Error::new(err),
        }
    }
}

impl std::fmt::Debug for SqlVaultCredentialSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqlVaultCredentialSource")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl CredentialSource for SqlVaultCredentialSource {
    async fn credentials(&self, provider: &CatalogProvider) -> Result<Credentials, ResolveError> {
        for _ in 0..2 {
            let before = self
                .store
                .snapshot()
                .await
                .map_err(|err| Self::store_error(provider.id(), err))?;
            let has_oauth = before
                .entries()
                .values()
                .any(|entry| entry.secret_type == SecretType::Oauth);
            if !has_oauth {
                // Only OAuth resolution can write back (token refresh); with no
                // OAuth secrets, skip the snapshot clones and CAS machinery.
                return self.source_for_snapshot(before).credentials(provider).await;
            }
            let source = self.source_for_snapshot(before.clone());
            let credentials = source.credentials(provider).await?;
            let after = source.snapshot().await;
            if self
                .persist_oauth_refreshes(&before, &after)
                .await
                .map_err(|err| Self::store_error(provider.id(), err))?
            {
                return Ok(credentials);
            }
        }
        Err(ResolveError::RefreshFailed {
            provider: provider.id().clone(),
            source:   anyhow::anyhow!("OAuth credential changed concurrently during refresh"),
        })
    }

    async fn configured_providers(&self, catalog: &Catalog) -> Vec<ProviderId> {
        let snapshot = match self.store.snapshot().await {
            Ok(snapshot) => snapshot,
            Err(err) => {
                error!(error = ?err, "Failed to load configured providers from secret store");
                return Vec::new();
            }
        };
        self.source_for_snapshot(snapshot)
            .configured_providers(catalog)
            .await
    }
}
