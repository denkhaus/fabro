//! Fabro's model resolver.
//!
//! lithos's [`CatalogResolver`] resolves selectors against the whole catalog.
//! Fabro layers policy on top: providers and models marked disabled in
//! `metadata.fabro` are unreachable through every selector shape (explicit
//! `provider/model`, alias, provider default, and the global `default`), and a
//! stand-in provider answers for the provider it stands in for when that
//! provider has no credentials of its own.

use std::collections::BTreeSet;

use fabro_types::catalog_policy;
use lithos_llm::catalog::{Catalog, CatalogProvider, ProviderId};
use lithos_llm::resolver::{
    AvailableProviders, CatalogResolver, ModelResolver, ModelSelectionError, ResolvedRoute,
};
use lithos_llm::types::Request;

/// Rejects disabled providers and models after catalog resolution.
#[derive(Clone, Copy, Debug, Default)]
pub struct FabroResolver;

impl FabroResolver {
    fn enabled_available(catalog: &Catalog, available: &AvailableProviders) -> AvailableProviders {
        AvailableProviders::new(
            available
                .iter()
                .filter(|id| {
                    catalog.provider_by_id(id).is_some_and(|provider| {
                        catalog_policy::provider_policy(provider).is_enabled()
                    })
                })
                .cloned()
                .collect::<BTreeSet<_>>(),
        )
    }

    /// The provider that stands in for `provider`, when one is available.
    fn stand_in<'a>(
        catalog: &'a Catalog,
        provider: &ProviderId,
        available: &AvailableProviders,
    ) -> Option<&'a CatalogProvider> {
        catalog.providers().find(|candidate| {
            available.contains(candidate.id())
                && catalog_policy::provider_policy(candidate)
                    .stands_in_for
                    .as_deref()
                    == Some(provider.as_str())
        })
    }

    fn check_route(route: ResolvedRoute) -> Result<ResolvedRoute, ModelSelectionError> {
        if !catalog_policy::provider_policy(route.provider()).is_enabled() {
            return Err(ModelSelectionError::ProviderUnavailable {
                provider: route.provider().id().clone(),
            });
        }
        if !catalog_policy::model_policy(route.model()).is_enabled() {
            return Err(ModelSelectionError::ModelNotFound {
                selector: route.handle().to_string(),
            });
        }
        Ok(route)
    }
}

impl ModelResolver for FabroResolver {
    fn resolve(
        &self,
        request: &Request,
        catalog: &Catalog,
        available: &AvailableProviders,
    ) -> Result<ResolvedRoute, ModelSelectionError> {
        let available = Self::enabled_available(catalog, available);
        match CatalogResolver.resolve(request, catalog, &available) {
            Ok(route) => Self::check_route(route),
            Err(ModelSelectionError::ProviderUnavailable { provider }) => {
                let Some(stand_in) = Self::stand_in(catalog, &provider, &available) else {
                    return Err(ModelSelectionError::ProviderUnavailable { provider });
                };
                let selector = request.model();
                let model_selector = selector
                    .split_once('/')
                    .map(|(_, model)| model)
                    .filter(|_| selector != provider.as_str());
                let rerouted_selector = match model_selector {
                    Some(model) => format!("{}/{model}", stand_in.id()),
                    None => stand_in.id().to_string(),
                };
                let rerouted = request
                    .clone()
                    .into_builder()
                    .model(rerouted_selector)
                    .build()
                    .map_err(|_| ModelSelectionError::ProviderUnavailable {
                        provider: provider.clone(),
                    })?;
                CatalogResolver
                    .resolve(&rerouted, catalog, &available)
                    .and_then(Self::check_route)
            }
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use lithos_llm::types::{Message, Role};

    use super::*;
    use crate::test_support::test_catalog;

    fn request(model: &str) -> Request {
        Request::builder()
            .model(model)
            .message(Message::text(Role::User, "hi"))
            .build()
            .unwrap()
    }

    fn resolve(
        catalog: &Catalog,
        model: &str,
        available: &[&str],
    ) -> Result<String, ModelSelectionError> {
        let available = AvailableProviders::new(available.iter().map(|id| ProviderId::new(*id)));
        FabroResolver
            .resolve(&request(model), catalog, &available)
            .map(|route| route.handle().to_string())
    }

    #[test]
    fn disabled_model_on_enabled_provider_stays_unreachable() {
        let catalog = Catalog::builder()
            .with_builtin()
            .toml_layer("policy", crate::FABRO_POLICY_TOML)
            .unwrap()
            .toml_layer(
                "test",
                r#"
schema_version = 1
[providers.openai.models."gpt-5.4".metadata.fabro]
enabled = false
"#,
            )
            .unwrap()
            .build()
            .unwrap();
        for selector in ["openai/gpt-5.4", "gpt-5.4", "codex"] {
            let error = resolve(&catalog, selector, &["openai"]).unwrap_err();
            assert!(
                matches!(error, ModelSelectionError::ModelNotFound { .. }),
                "{selector}: {error:?}"
            );
        }
        assert_eq!(
            resolve(&catalog, "gpt-5.4-mini", &["openai"]).unwrap(),
            "openai/gpt-5.4-mini"
        );
    }

    #[test]
    fn disabled_provider_is_unavailable_even_when_credentialed() {
        let catalog = test_catalog();
        let error = resolve(&catalog, "bedrock/claude-sonnet-5", &["bedrock"]).unwrap_err();
        assert!(matches!(
            error,
            ModelSelectionError::ProviderUnavailable { .. }
        ));
        let error = resolve(&catalog, "default", &["bedrock"]).unwrap_err();
        assert!(matches!(error, ModelSelectionError::NoDefaultModel));
    }

    #[test]
    fn codex_stands_in_for_openai_without_an_api_key() {
        let catalog = test_catalog();
        assert_eq!(
            resolve(&catalog, "openai/gpt-5.4-mini", &["openai-codex"]).unwrap(),
            "openai-codex/gpt-5.4-mini"
        );
        assert_eq!(
            resolve(&catalog, "openai", &["openai-codex"]).unwrap(),
            "openai-codex/gpt-5.6-sol"
        );
        assert_eq!(
            resolve(&catalog, "openai/gpt-5.4-mini", &["openai", "openai-codex"]).unwrap(),
            "openai/gpt-5.4-mini",
            "the real provider wins when it is ready"
        );
    }

    #[test]
    fn enabled_routes_resolve_like_lithos() {
        let catalog = test_catalog();
        assert_eq!(
            resolve(&catalog, "sonnet", &["anthropic", "openai"]).unwrap(),
            "anthropic/claude-sonnet-5"
        );
        assert_eq!(
            resolve(&catalog, "default", &["openai"]).unwrap(),
            "openai/gpt-5.6-sol"
        );
    }
}
