//! Catalog construction and Fabro-policy queries.
//!
//! Layer order is fixed: lithos built-ins, then Fabro's policy layer, then the
//! operator's `[llm]` overlay. Every query here reads Fabro policy from the
//! `metadata.fabro` namespace and never bypasses `enabled`.

use std::collections::{BTreeMap, HashSet};

use fabro_config::LlmLayer;
use fabro_static::EnvVars;
use fabro_types::catalog_policy::{self, ModelPolicy, ProviderPolicy};
use fabro_types::{AgentProfileKind, Cost, ModelId, ModelRef, ProviderId, TokenCounts};
use lithos_llm::catalog::{Catalog, CatalogError, CatalogModel, CatalogProvider};
use lithos_llm::resolver::ResolvedRoute;

/// Fabro's policy layer, applied above the lithos built-ins.
pub const FABRO_POLICY_TOML: &str = include_str!("../catalog/fabro-policy.toml");

/// Builds the effective catalog.
///
/// `env_lookup` supplies `OPENAI_BASE_URL`, the one environment override
/// Fabro honors: it repoints the `openai` provider so test doubles and
/// gateways can stand in for the real API without editing settings.
pub fn build_catalog(
    overlay: &LlmLayer,
    env_lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<Catalog, CatalogError> {
    let mut builder = Catalog::builder()
        .with_builtin()
        .toml_layer("fabro-policy.toml", FABRO_POLICY_TOML)?;
    if !overlay.is_empty() {
        let mut document = overlay.to_overlay_toml();
        document.insert_str(0, "schema_version = 1\n");
        builder = builder.toml_layer("settings [llm]", &document)?;
    }
    if let Some(base_url) = env_lookup(EnvVars::OPENAI_BASE_URL) {
        let document = format!(
            "schema_version = 1\n[providers.openai]\nbase_url = {}\n",
            toml::Value::String(base_url.trim_end_matches("/v1").to_string())
        );
        builder = builder.toml_layer("OPENAI_BASE_URL", &document)?;
    }
    builder.build()
}

/// A provider with its Fabro policy attached.
#[derive(Debug, Clone)]
pub struct ProviderEntry<'a> {
    pub provider: &'a CatalogProvider,
    pub policy:   ProviderPolicy,
}

/// A model with its Fabro policy attached.
#[derive(Debug, Clone)]
pub struct ModelEntry<'a> {
    pub provider: &'a CatalogProvider,
    pub model:    &'a CatalogModel,
    pub policy:   ModelPolicy,
}

impl ModelEntry<'_> {
    /// Whether requests to this model reason when no effort is requested.
    ///
    /// Fabro policy can state it outright. Otherwise a model that supports
    /// reasoning and takes named effort levels reasons by default, while one
    /// that needs an explicit thinking budget does not.
    #[must_use]
    pub fn reasons_by_default(&self) -> bool {
        self.policy.reasoning_by_default.unwrap_or_else(|| {
            self.model.capabilities().reasoning().is_supported()
                && self.model.protocol_options().reasoning_effort_levels
        })
    }

    #[must_use]
    pub fn agent_profile(&self) -> AgentProfileKind {
        catalog_policy::effective_agent_profile(self.provider, self.model)
    }
}

/// The catalog with no operator overlay: lithos built-ins plus Fabro policy.
///
/// Used where no settings file is in play, such as the standalone hook
/// runner. Servers and the CLI build from the operator's `[llm]` overlay
/// with [`build_catalog`] instead.
#[must_use]
pub fn default_catalog() -> Catalog {
    build_catalog(&LlmLayer::default(), &|_| None)
        .expect("the built-in catalog and Fabro policy layer always build")
}

/// Estimates the catalog cost of `usage` on `model`, when the catalog prices
/// that route. Passthrough models and unknown providers have no price.
#[must_use]
pub fn estimate_cost(catalog: &Catalog, model: &ModelRef, usage: TokenCounts) -> Option<Cost> {
    let entry = model_on_provider(catalog, model.provider.as_str(), model.model_id.as_str())?;
    ResolvedRoute::try_new(entry.provider.clone(), entry.model.clone())
        .ok()?
        .estimate_cost(usage, model.speed)
}

/// Enabled providers, highest priority first, ties broken by id.
#[must_use]
pub fn enabled_providers(catalog: &Catalog) -> Vec<ProviderEntry<'_>> {
    let mut providers: Vec<_> = catalog
        .providers()
        .map(|provider| ProviderEntry {
            provider,
            policy: catalog_policy::provider_policy(provider),
        })
        .filter(|entry| entry.policy.is_enabled())
        .collect();
    providers.sort_by(|left, right| {
        right
            .provider
            .priority()
            .cmp(&left.provider.priority())
            .then_with(|| left.provider.id().cmp(right.provider.id()))
    });
    providers
}

/// Enabled providers that Fabro lists to operators. Stand-in providers such
/// as `openai-codex` route requests but are not offerings of their own.
#[must_use]
pub fn listed_providers(catalog: &Catalog) -> Vec<ProviderEntry<'_>> {
    enabled_providers(catalog)
        .into_iter()
        .filter(|entry| entry.policy.stands_in_for.is_none())
        .collect()
}

/// The ids of every enabled provider.
#[must_use]
pub fn enabled_provider_ids(catalog: &Catalog) -> HashSet<ProviderId> {
    enabled_providers(catalog)
        .into_iter()
        .map(|entry| entry.provider.id().clone())
        .collect()
}

/// Looks up an enabled provider by id or alias.
#[must_use]
pub fn provider<'a>(catalog: &'a Catalog, selector: &str) -> Option<ProviderEntry<'a>> {
    let provider = catalog.provider(selector).ok()?;
    let policy = catalog_policy::provider_policy(provider);
    policy
        .is_enabled()
        .then_some(ProviderEntry { provider, policy })
}

/// Canonicalizes a provider id or alias to its catalog id, when enabled.
#[must_use]
pub fn canonical_provider_id(catalog: &Catalog, selector: &str) -> Option<ProviderId> {
    provider(catalog, selector).map(|entry| entry.provider.id().clone())
}

/// Enabled models of an enabled provider, in catalog order.
#[must_use]
pub fn provider_models(provider: &CatalogProvider) -> Vec<ModelEntry<'_>> {
    provider
        .models()
        .map(|model| ModelEntry {
            provider,
            model,
            policy: catalog_policy::model_policy(model),
        })
        .filter(|entry| entry.policy.is_enabled())
        .collect()
}

/// Every enabled model across listed providers, provider priority order.
#[must_use]
pub fn models(catalog: &Catalog) -> Vec<ModelEntry<'_>> {
    listed_providers(catalog)
        .into_iter()
        .flat_map(|entry| provider_models(entry.provider))
        .collect()
}

/// Finds an enabled model on an enabled provider by id, alias, or wire id.
#[must_use]
pub fn model_on_provider<'a>(
    catalog: &'a Catalog,
    provider_selector: &str,
    model_selector: &str,
) -> Option<ModelEntry<'a>> {
    let entry = provider(catalog, provider_selector)?;
    // lithos matches ids and aliases. The provider's wire id (an aggregator's
    // `vendor/model`) is accepted too, so a selector copied from the
    // provider's own listing lands on the catalog row instead of passing
    // through unknown.
    let model = entry.provider.model(model_selector).or_else(|| {
        entry
            .provider
            .models()
            .find(|model| model.api_model() == model_selector)
    })?;
    let policy = catalog_policy::model_policy(model);
    policy.is_enabled().then_some(ModelEntry {
        provider: entry.provider,
        model,
        policy,
    })
}

/// Enabled models matching `selector` by id or alias, ordered like lithos
/// selection: exact ids before aliases, then provider priority.
#[must_use]
pub fn models_matching<'a>(catalog: &'a Catalog, selector: &str) -> Vec<ModelEntry<'a>> {
    let mut matches: Vec<_> = enabled_providers(catalog)
        .into_iter()
        .flat_map(|entry| provider_models(entry.provider))
        .filter(|entry| {
            entry.model.id().as_str() == selector
                || entry.model.aliases().iter().any(|alias| alias == selector)
        })
        .collect();
    matches.sort_by_key(|entry| entry.model.id().as_str() != selector);
    matches
}

/// Whether `selector` names an enabled model on any enabled provider.
#[must_use]
pub fn is_model_selector(catalog: &Catalog, selector: &str) -> bool {
    !models_matching(catalog, selector).is_empty()
}

/// Whether `selector` names an enabled provider.
#[must_use]
pub fn is_provider_selector(catalog: &Catalog, selector: &str) -> bool {
    provider(catalog, selector).is_some()
}

/// The enabled default model of an enabled provider.
#[must_use]
pub fn default_model<'a>(catalog: &'a Catalog, provider_selector: &str) -> Option<ModelEntry<'a>> {
    let entry = provider(catalog, provider_selector)?;
    let default = entry.provider.default_model()?;
    model_on_provider(catalog, entry.provider.id().as_str(), default)
}

/// The model Fabro probes a provider with: the `probe` model, else the
/// provider default.
#[must_use]
pub fn probe_model<'a>(catalog: &'a Catalog, provider_selector: &str) -> Option<ModelEntry<'a>> {
    let entry = provider(catalog, provider_selector)?;
    provider_models(entry.provider)
        .into_iter()
        .find(|model| model.policy.probe)
        .or_else(|| default_model(catalog, provider_selector))
}

/// The default model across `ready` providers: the highest-priority ready
/// provider's default. Falls back to any enabled provider's default when no
/// provider is ready, so callers always have a model to name.
#[must_use]
pub fn default_for_ready<'a>(
    catalog: &'a Catalog,
    ready: &HashSet<ProviderId>,
) -> Option<ModelEntry<'a>> {
    let providers = enabled_providers(catalog);
    providers
        .iter()
        .filter(|entry| ready.contains(entry.provider.id()))
        .chain(providers.iter())
        .find_map(|entry| default_model(catalog, entry.provider.id().as_str()))
}

/// The small utility model across `ready` providers: the first
/// `small_default` model in provider priority order, else the ready default.
#[must_use]
pub fn small_default_for_ready<'a>(
    catalog: &'a Catalog,
    ready: &HashSet<ProviderId>,
) -> Option<ModelEntry<'a>> {
    let providers = enabled_providers(catalog);
    providers
        .iter()
        .filter(|entry| ready.contains(entry.provider.id()))
        .flat_map(|entry| provider_models(entry.provider))
        .find(|model| model.policy.small_default)
        .or_else(|| default_for_ready(catalog, ready))
}

/// Canonicalizes a model selector to a catalog model id, preferring
/// `provider`'s offering. Unknown selectors pass through verbatim so
/// passthrough models keep their names.
#[must_use]
pub fn canonical_model_id(catalog: &Catalog, provider: &ProviderId, selector: &str) -> String {
    model_on_provider(catalog, provider.as_str(), selector)
        .map(|entry| entry.model.id().to_string())
        .or_else(|| {
            models_matching(catalog, selector)
                .first()
                .map(|entry| entry.model.id().to_string())
        })
        .unwrap_or_else(|| selector.to_string())
}

/// The agent profile for a route. Unknown (passthrough) models take the
/// provider default.
#[must_use]
pub fn agent_profile(
    catalog: &Catalog,
    provider_selector: &str,
    model_selector: Option<&str>,
) -> Option<AgentProfileKind> {
    let entry = provider(catalog, provider_selector)?;
    let model = model_selector.and_then(|selector| entry.provider.model(selector));
    Some(match model {
        Some(model) => catalog_policy::effective_agent_profile(entry.provider, model),
        None => entry
            .policy
            .agent_profile
            .unwrap_or_else(|| catalog_policy::default_agent_profile(entry.provider)),
    })
}

/// The `target` provider's model closest to `reference` in capability and
/// input price, for provider-level fallbacks.
#[must_use]
pub fn closest_model<'a>(
    catalog: &'a Catalog,
    target: &str,
    reference: &CatalogModel,
) -> Option<ModelEntry<'a>> {
    let target = provider(catalog, target)?;
    let reference_caps = reference.capabilities();
    let reference_price = reference
        .pricing()
        .and_then(|pricing| pricing.input_usd_micros_per_million)
        .unwrap_or(0);
    provider_models(target.provider)
        .into_iter()
        .filter(|entry| {
            let caps = entry.model.capabilities();
            caps.tools().is_supported() == reference_caps.tools().is_supported()
                && caps.images().is_supported() == reference_caps.images().is_supported()
                && caps.reasoning().is_supported() == reference_caps.reasoning().is_supported()
        })
        .min_by_key(|entry| {
            let price = entry
                .model
                .pricing()
                .and_then(|pricing| pricing.input_usd_micros_per_million)
                .unwrap_or(0);
            price.abs_diff(reference_price)
        })
}

/// Model ids grouped by provider, for diagnostics and documentation.
#[must_use]
pub fn model_ids_by_provider(catalog: &Catalog) -> BTreeMap<ProviderId, Vec<ModelId>> {
    listed_providers(catalog)
        .into_iter()
        .map(|entry| {
            (
                entry.provider.id().clone(),
                provider_models(entry.provider)
                    .into_iter()
                    .map(|model| model.model.id().clone())
                    .collect(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_catalog;

    #[test]
    fn policy_layer_builds_over_the_builtins() {
        let catalog = test_catalog();
        let ids: Vec<_> = enabled_providers(&catalog)
            .iter()
            .map(|entry| entry.provider.id().to_string())
            .collect();
        assert_eq!(ids[0], "anthropic");
        assert!(ids.contains(&"openai".to_string()));
        assert!(
            !ids.contains(&"bedrock".to_string()),
            "bedrock ships disabled"
        );
        assert!(
            !listed_providers(&catalog)
                .iter()
                .any(|entry| entry.provider.id().as_str() == "openai-codex"),
            "stand-in providers are not listed"
        );
    }

    #[test]
    fn operator_overlay_applies_last() {
        let overlay = LlmLayer(
            toml::from_str(
                r"
[providers.openai]
priority = 500
[providers.openai.metadata.fabro]
enabled = false
",
            )
            .unwrap(),
        );
        let catalog = build_catalog(&overlay, &|_| None).unwrap();
        assert!(provider(&catalog, "openai").is_none());
        assert_eq!(
            catalog.provider("openai").unwrap().priority(),
            500,
            "overlay values win over the policy layer"
        );
    }

    #[test]
    fn openai_base_url_env_repoints_the_openai_provider() {
        let catalog = build_catalog(&LlmLayer::default(), &|name| {
            (name == EnvVars::OPENAI_BASE_URL).then(|| "http://127.0.0.1:1234/v1".to_string())
        })
        .unwrap();
        assert_eq!(
            catalog.provider("openai").unwrap().base_url(),
            "http://127.0.0.1:1234"
        );
    }

    #[test]
    fn probe_and_small_default_follow_policy() {
        let catalog = test_catalog();
        assert_eq!(
            probe_model(&catalog, "openai").unwrap().model.id().as_str(),
            "gpt-5.4-mini"
        );
        assert_eq!(
            probe_model(&catalog, "anthropic")
                .unwrap()
                .model
                .id()
                .as_str(),
            "claude-haiku-4.5"
        );
        let ready = HashSet::from([ProviderId::new("openai")]);
        assert_eq!(
            small_default_for_ready(&catalog, &ready)
                .unwrap()
                .model
                .id()
                .as_str(),
            "gpt-5.4-mini"
        );
        assert_eq!(
            default_for_ready(&catalog, &ready)
                .unwrap()
                .model
                .id()
                .as_str(),
            "gpt-5.6-sol"
        );
        assert_eq!(
            default_for_ready(&catalog, &HashSet::new())
                .unwrap()
                .provider
                .id()
                .as_str(),
            "anthropic"
        );
    }

    #[test]
    fn selectors_resolve_aliases_and_canonical_ids() {
        let catalog = test_catalog();
        assert!(is_model_selector(&catalog, "sonnet"));
        assert!(is_model_selector(&catalog, "gpt-5.4-mini"));
        assert!(!is_model_selector(&catalog, "nope"));
        assert_eq!(
            canonical_model_id(&catalog, &ProviderId::new("openai"), "codex"),
            "gpt-5.4"
        );
        assert_eq!(
            canonical_model_id(&catalog, &ProviderId::new("openai"), "unknown-model"),
            "unknown-model"
        );
        assert_eq!(
            agent_profile(&catalog, "openai", Some("gpt-5.6-sol")),
            Some(AgentProfileKind::Gpt56)
        );
        assert_eq!(
            agent_profile(&catalog, "moonshot", None),
            Some(AgentProfileKind::OpenAi)
        );
        assert_eq!(
            agent_profile(&catalog, "moonshot", Some("kimi-k3")),
            Some(AgentProfileKind::Kimi)
        );
    }
}
