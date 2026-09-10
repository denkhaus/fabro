//! Fabro-managed inventory over a sandbox-driver provider.
//!
//! Lists and looks up the sandboxes fabro created, identified by fabro's
//! own `sh.fabro.managed` label. The driver marks every sandbox it creates
//! with its own label too, but that covers every application on the same
//! daemon or account; the provider is connected through the driver's
//! ownership scope, which lists only fabro's sandboxes and refuses to
//! attach to or delete any other.

use std::sync::Arc;

use async_trait::async_trait;
use fabro_types::settings::server::ServerSandboxProviderSettings;
use fabro_types::{SandboxInfo, SandboxProviderKind};
use sandbox_driver::{
    Error as DriverError, OwnedProvider, SandboxFilter, SandboxId,
    SandboxProvider as DriverProvider,
};
use tokio::sync::OnceCell;

use super::SandboxProvider;
use crate::driver::{ConnectedProvider, ProviderConnectOptions, connect_provider};
use crate::{details, managed_labels};

/// How the driver provider behind the inventory is obtained.
enum Connection {
    Connected(Arc<dyn DriverProvider>),
    /// Connected on first use, so a registry can be assembled synchronously
    /// and a provider that is down surfaces as a lookup error rather than a
    /// startup failure.
    Lazy(Box<LazyConnection>),
}

struct LazyConnection {
    settings: ServerSandboxProviderSettings,
    options:  ProviderConnectOptions,
    provider: OnceCell<Arc<dyn DriverProvider>>,
}

pub struct DriverInventoryProvider {
    kind:       SandboxProviderKind,
    connection: Connection,
}

impl DriverInventoryProvider {
    #[must_use]
    pub fn new(connected: ConnectedProvider) -> Self {
        Self {
            kind:       connected.kind,
            connection: Connection::Connected(owned(connected.provider)),
        }
    }

    /// An inventory over a provider connected through
    /// [`connect_provider`] on first use.
    #[must_use]
    pub fn lazy(
        kind: SandboxProviderKind,
        settings: ServerSandboxProviderSettings,
        options: ProviderConnectOptions,
    ) -> Self {
        Self {
            kind,
            connection: Connection::Lazy(Box::new(LazyConnection {
                settings,
                options,
                provider: OnceCell::new(),
            })),
        }
    }

    async fn provider(&self) -> crate::Result<&Arc<dyn DriverProvider>> {
        match &self.connection {
            Connection::Connected(provider) => Ok(provider),
            Connection::Lazy(lazy) => {
                lazy.provider
                    .get_or_try_init(|| async {
                        connect_provider(&self.kind, &lazy.settings, &lazy.options)
                            .await
                            .map(|connected| owned(connected.provider))
                            .map_err(|error| {
                                crate::Error::context(
                                    format!("Failed to connect to the {} provider", self.kind),
                                    error,
                                )
                            })
                    })
                    .await
            }
        }
    }

    async fn describe_managed(
        &self,
        id: &str,
    ) -> crate::Result<Option<sandbox_driver::SandboxStatus>> {
        // An id the driver cannot even name is not one of ours.
        let Ok(sandbox_id) = SandboxId::try_new(id) else {
            return Ok(None);
        };
        let handle = match self.provider().await?.attach(&sandbox_id, None).await {
            Ok(handle) => handle,
            // Unknown to the provider, or not fabro's: neither is in the
            // inventory.
            Err(DriverError::NotFound { .. } | DriverError::NotOwned { .. }) => return Ok(None),
            Err(error) => {
                return Err(crate::Error::context(
                    format!("Failed to look up {} sandbox '{id}'", self.kind),
                    error,
                ));
            }
        };
        let status = handle.describe().await.map_err(|error| {
            crate::Error::context(
                format!("Failed to describe {} sandbox '{id}'", self.kind),
                error,
            )
        })?;
        if status.state == sandbox_driver::SandboxState::Deleted {
            return Ok(None);
        }
        Ok(Some(status))
    }
}

/// The provider narrowed to fabro's sandboxes.
fn owned(provider: Arc<dyn DriverProvider>) -> Arc<dyn DriverProvider> {
    Arc::new(OwnedProvider::new(
        provider,
        managed_labels::ownership(None),
    ))
}

#[async_trait]
impl SandboxProvider for DriverInventoryProvider {
    fn kind(&self) -> SandboxProviderKind {
        self.kind.clone()
    }

    async fn list(&self) -> crate::Result<Vec<SandboxInfo>> {
        let statuses = self
            .provider()
            .await?
            .list(&SandboxFilter::default())
            .await
            .map_err(|error| {
                crate::Error::context(format!("Failed to list {} sandboxes", self.kind), error)
            })?;
        Ok(statuses
            .iter()
            .map(|status| details::info_from_status(&self.kind, status))
            .collect())
    }

    async fn get(&self, id: &str) -> crate::Result<Option<SandboxInfo>> {
        Ok(self
            .describe_managed(id)
            .await?
            .map(|status| details::info_from_status(&self.kind, &status)))
    }

    async fn delete(&self, id: &str) -> crate::Result<()> {
        // Missing or already deleted is an idempotent success; the scope
        // refuses a sandbox that is not fabro's, which must never be
        // deleted here.
        let Ok(sandbox_id) = SandboxId::try_new(id) else {
            return Ok(());
        };
        match self.provider().await?.delete(&sandbox_id, None).await {
            Ok(()) => Ok(()),
            Err(DriverError::NotOwned { .. }) => Err(crate::Error::message(format!(
                "Refusing to delete {} sandbox '{id}' because it is missing label {}={}",
                self.kind,
                managed_labels::MANAGED_LABEL,
                managed_labels::MANAGED_LABEL_VALUE
            ))),
            Err(error) => Err(crate::Error::context(
                format!("Failed to delete {} sandbox '{id}'", self.kind),
                error,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use sandbox_driver::{SandboxSource, SandboxSpec};
    use sandbox_driver_host::HostProvider;

    use super::*;

    fn inventory() -> (DriverInventoryProvider, Arc<HostProvider>) {
        let host = Arc::new(HostProvider::new());
        let provider = DriverInventoryProvider::new(ConnectedProvider {
            kind:     SandboxProviderKind::try_new("host").unwrap(),
            provider: host.clone(),
        });
        (provider, host)
    }

    #[tokio::test]
    async fn lists_and_deletes_only_fabro_managed_sandboxes() {
        let (inventory, host) = inventory();
        let ours = host
            .create(
                &SandboxSpec::new(SandboxSource::HostDirectory)
                    .label(managed_labels::MANAGED_LABEL, "true"),
                None,
            )
            .await
            .unwrap();
        let theirs = host
            .create(&SandboxSpec::new(SandboxSource::HostDirectory), None)
            .await
            .unwrap();

        let listed = inventory.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, ours.id().as_str());
        assert_eq!(listed[0].provider.as_str(), "host");
        assert!(inventory.get(ours.id().as_str()).await.unwrap().is_some());
        assert!(inventory.get(theirs.id().as_str()).await.unwrap().is_none());

        let refused = inventory.delete(theirs.id().as_str()).await.unwrap_err();
        assert!(
            refused.to_string().contains("Refusing to delete"),
            "{refused}"
        );
        inventory.delete(ours.id().as_str()).await.unwrap();
        assert!(inventory.get(ours.id().as_str()).await.unwrap().is_none());
        inventory.delete(ours.id().as_str()).await.unwrap();
        inventory.delete("never-existed").await.unwrap();
    }
}
