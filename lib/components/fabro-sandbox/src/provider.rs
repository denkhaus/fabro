#[cfg(feature = "daytona")]
pub mod daytona;
pub mod driver;

use std::sync::Arc;

use async_trait::async_trait;
use fabro_types::{
    SandboxInfo, SandboxListMeta, SandboxListResponse, SandboxProviderKind,
    SandboxProviderLookupError,
};
use fabro_util::error::collect_chain;
use futures::future::join_all;

#[async_trait]
pub trait SandboxProvider: Send + Sync {
    fn kind(&self) -> SandboxProviderKind;

    async fn list(&self) -> crate::Result<Vec<SandboxInfo>>;
    async fn get(&self, id: &str) -> crate::Result<Option<SandboxInfo>>;
    async fn delete(&self, id: &str) -> crate::Result<()>;
}

#[derive(Clone, Default)]
pub struct SandboxProviderRegistry {
    providers: Vec<Arc<dyn SandboxProvider>>,
}

impl SandboxProviderRegistry {
    pub fn new(providers: Vec<Arc<dyn SandboxProvider>>) -> Self {
        Self { providers }
    }

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn providers(&self) -> &[Arc<dyn SandboxProvider>] {
        &self.providers
    }

    pub async fn list_managed(&self) -> SandboxListResponse {
        let results = join_all(
            self.providers
                .iter()
                .map(|provider| async move { (provider.kind(), provider.list().await) }),
        )
        .await;

        let mut data = Vec::new();
        let mut provider_errors = Vec::new();
        for (kind, result) in results {
            match result {
                Ok(mut sandboxes) => data.append(&mut sandboxes),
                Err(err) => provider_errors.push(provider_error(kind, &err)),
            }
        }

        SandboxListResponse {
            data,
            meta: SandboxListMeta { provider_errors },
        }
    }

    pub async fn get_managed_by_native_id(
        &self,
        id: &str,
    ) -> Result<SandboxInfo, SandboxLookupError> {
        let results = join_all(
            self.providers
                .iter()
                .map(|provider| async move { (provider.kind(), provider.get(id).await) }),
        )
        .await;

        let mut matches = Vec::new();
        let mut provider_errors = Vec::new();
        for (kind, result) in results {
            match result {
                Ok(Some(sandbox)) => matches.push(sandbox),
                Ok(None) => {}
                Err(err) => provider_errors.push(provider_error(kind, &err)),
            }
        }

        match matches.len() {
            1 => Ok(matches.remove(0)),
            0 if provider_errors.is_empty() => {
                Err(SandboxLookupError::NotFound { id: id.to_string() })
            }
            0 => Err(SandboxLookupError::ProviderUnavailable {
                id: id.to_string(),
                provider_errors,
            }),
            _ => Err(SandboxLookupError::Conflict {
                id:        id.to_string(),
                providers: matches
                    .into_iter()
                    .map(|sandbox| sandbox.provider)
                    .collect(),
            }),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxLookupError {
    #[error("sandbox '{id}' was not found by any configured provider")]
    NotFound { id: String },
    #[error("sandbox '{id}' matched more than one configured provider")]
    Conflict {
        id:        String,
        providers: Vec<SandboxProviderKind>,
    },
    #[error("sandbox '{id}' could not be found definitively because one or more providers failed")]
    ProviderUnavailable {
        id:              String,
        provider_errors: Vec<SandboxProviderLookupError>,
    },
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LocalSandboxProvider;

#[async_trait]
impl SandboxProvider for LocalSandboxProvider {
    fn kind(&self) -> SandboxProviderKind {
        SandboxProviderKind::LOCAL
    }

    async fn list(&self) -> crate::Result<Vec<SandboxInfo>> {
        Ok(Vec::new())
    }

    async fn get(&self, _id: &str) -> crate::Result<Option<SandboxInfo>> {
        Ok(None)
    }

    async fn delete(&self, _id: &str) -> crate::Result<()> {
        Ok(())
    }
}

fn provider_error(
    provider: SandboxProviderKind,
    err: &(dyn std::error::Error + 'static),
) -> SandboxProviderLookupError {
    SandboxProviderLookupError {
        provider,
        message: collect_chain(err).join(": "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        FakeGet, FakeList, FakeSandboxProvider, fake_registry, fake_sandbox_info,
    };

    #[tokio::test]
    async fn list_returns_aggregate_data_from_successful_providers() {
        let docker = fake_sandbox_info(SandboxProviderKind::DOCKER, "docker-1");
        let daytona = fake_sandbox_info(SandboxProviderKind::DAYTONA, "daytona-1");
        let registry = fake_registry(vec![
            FakeSandboxProvider::new(
                SandboxProviderKind::DOCKER,
                FakeList::Ok(vec![docker.clone()]),
                FakeGet::Missing,
            ),
            FakeSandboxProvider::new(
                SandboxProviderKind::DAYTONA,
                FakeList::Ok(vec![daytona.clone()]),
                FakeGet::Missing,
            ),
        ]);

        let response = registry.list_managed().await;

        assert_eq!(response.data, vec![docker, daytona]);
        assert!(response.meta.provider_errors.is_empty());
    }

    #[tokio::test]
    async fn list_includes_provider_error_metadata_when_one_provider_fails() {
        let docker = fake_sandbox_info(SandboxProviderKind::DOCKER, "docker-1");
        let registry = fake_registry(vec![
            FakeSandboxProvider::new(
                SandboxProviderKind::DOCKER,
                FakeList::Ok(vec![docker.clone()]),
                FakeGet::Missing,
            ),
            FakeSandboxProvider::new(
                SandboxProviderKind::DAYTONA,
                FakeList::Err("daytona unavailable"),
                FakeGet::Missing,
            ),
        ]);

        let response = registry.list_managed().await;

        assert_eq!(response.data, vec![docker]);
        assert_eq!(response.meta.provider_errors, vec![
            SandboxProviderLookupError {
                provider: SandboxProviderKind::DAYTONA,
                message:  "daytona unavailable".to_string(),
            }
        ]);
    }

    #[tokio::test]
    async fn get_returns_one_matching_sandbox() {
        let docker = fake_sandbox_info(SandboxProviderKind::DOCKER, "same-id");
        let registry = fake_registry(vec![
            FakeSandboxProvider::new(
                SandboxProviderKind::DOCKER,
                FakeList::Ok(Vec::new()),
                FakeGet::Found(Box::new(docker.clone())),
            ),
            FakeSandboxProvider::new(
                SandboxProviderKind::DAYTONA,
                FakeList::Ok(Vec::new()),
                FakeGet::Missing,
            ),
        ]);

        assert_eq!(
            registry.get_managed_by_native_id("same-id").await.unwrap(),
            docker
        );
    }

    #[tokio::test]
    async fn get_returns_not_found_when_all_providers_miss() {
        let registry = fake_registry(vec![
            FakeSandboxProvider::new(
                SandboxProviderKind::DOCKER,
                FakeList::Ok(Vec::new()),
                FakeGet::Missing,
            ),
            FakeSandboxProvider::new(
                SandboxProviderKind::DAYTONA,
                FakeList::Ok(Vec::new()),
                FakeGet::Missing,
            ),
        ]);

        let err = registry
            .get_managed_by_native_id("missing")
            .await
            .unwrap_err();

        assert!(matches!(err, SandboxLookupError::NotFound { id } if id == "missing"));
    }

    #[tokio::test]
    async fn get_returns_conflict_when_two_providers_match() {
        let registry = fake_registry(vec![
            FakeSandboxProvider::new(
                SandboxProviderKind::DOCKER,
                FakeList::Ok(Vec::new()),
                FakeGet::Found(Box::new(fake_sandbox_info(
                    SandboxProviderKind::DOCKER,
                    "same-id",
                ))),
            ),
            FakeSandboxProvider::new(
                SandboxProviderKind::DAYTONA,
                FakeList::Ok(Vec::new()),
                FakeGet::Found(Box::new(fake_sandbox_info(
                    SandboxProviderKind::DAYTONA,
                    "same-id",
                ))),
            ),
        ]);

        let err = registry
            .get_managed_by_native_id("same-id")
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            SandboxLookupError::Conflict { id, providers }
                if id == "same-id"
                    && providers == vec![SandboxProviderKind::DOCKER, SandboxProviderKind::DAYTONA]
        ));
    }

    #[tokio::test]
    async fn get_returns_provider_unavailable_when_no_match_and_one_provider_fails() {
        let registry = fake_registry(vec![
            FakeSandboxProvider::new(
                SandboxProviderKind::DOCKER,
                FakeList::Ok(Vec::new()),
                FakeGet::Missing,
            ),
            FakeSandboxProvider::new(
                SandboxProviderKind::DAYTONA,
                FakeList::Ok(Vec::new()),
                FakeGet::Err("daytona unavailable"),
            ),
        ]);

        let err = registry
            .get_managed_by_native_id("maybe-missing")
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            SandboxLookupError::ProviderUnavailable {
                id,
                provider_errors
            } if id == "maybe-missing"
                && provider_errors == vec![SandboxProviderLookupError {
                    provider: SandboxProviderKind::DAYTONA,
                    message: "daytona unavailable".to_string(),
                }]
        ));
    }
}
