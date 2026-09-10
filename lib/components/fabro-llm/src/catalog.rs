//! Catalog construction and the queries Fabro's dispatch boundaries share.
//!
//! Layer order is fixed: lithos built-ins, then the operator's `[llm]`
//! overlay. Provider and model facts, `enabled`, `stands_in_for`,
//! `small_default`, and `probe` are lithos core fields. The agent harness a
//! model expects lives in the shared `metadata.agent` namespace, which Pebble
//! reads too. Every query here skips disabled providers.

use std::collections::{BTreeMap, HashSet};

use fabro_config::LlmLayer;
use fabro_static::EnvVars;
use fabro_types::{AgentProfileKind, Cost, ModelId, ModelRef, ProviderId, TokenCounts};
use lithos_llm::catalog::{Catalog, CatalogError, CatalogModel, CatalogProvider, Metadata};
use lithos_llm::resolver::ResolvedRoute;
use serde::Deserialize;

/// The metadata namespace agent harnesses read.
const AGENT_METADATA_NAMESPACE: &str = "agent";

/// Builds the effective catalog.
///
/// `env_lookup` supplies `OPENAI_BASE_URL`, the one environment override
/// Fabro honors: it repoints the `openai` provider so test doubles and
/// gateways can stand in for the real API without editing settings.
pub fn build_catalog(
    overlay: &LlmLayer,
    env_lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<Catalog, CatalogError> {
    let mut builder = Catalog::builder().with_builtin();
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

/// The catalog with no operator overlay: the lithos built-ins.
///
/// Used where no settings file is in play, such as the standalone hook
/// runner. Servers and the CLI build from the operator's `[llm]` overlay
/// with [`build_catalog`] instead.
#[must_use]
pub fn default_catalog() -> Catalog {
    build_catalog(&LlmLayer::default(), &|_| None).expect("the built-in catalog always builds")
}

/// A model on the provider that offers it.
#[derive(Debug, Clone)]
pub struct ModelEntry<'a> {
    pub provider: &'a CatalogProvider,
    pub model:    &'a CatalogModel,
}

impl ModelEntry<'_> {
    /// Whether requests to this model reason when no effort is requested.
    ///
    /// The catalog can state it outright under `metadata.agent`. Otherwise a
    /// model that supports reasoning and takes named effort levels reasons by
    /// default, while one that needs an explicit thinking budget does not.
    #[must_use]
    pub fn reasons_by_default(&self) -> bool {
        agent_metadata(self.model.metadata())
            .reasoning_by_default
            .or(agent_metadata(self.provider.metadata()).reasoning_by_default)
            .unwrap_or_else(|| {
                self.model.capabilities().reasoning().is_supported()
                    && self.model.protocol_options().reasoning_effort_levels
            })
    }

    /// The agent harness this model runs under: the model's own answer, then
    /// the provider's, then the profile implied by the provider's adapter.
    #[must_use]
    pub fn agent_profile(&self) -> AgentProfileKind {
        agent_metadata(self.model.metadata())
            .profile
            .unwrap_or_else(|| provider_agent_profile(self.provider))
    }
}

/// The `metadata.agent` namespace on a catalog entry. Malformed metadata
/// falls back to the defaults; the lithos built-ins are validated in lithos.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct AgentMetadata {
    profile:              Option<AgentProfileKind>,
    reasoning_by_default: Option<bool>,
}

fn agent_metadata(metadata: &Metadata) -> AgentMetadata {
    metadata
        .namespace::<AgentMetadata>(AGENT_METADATA_NAMESPACE)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// The agent profile a provider's models run under unless a model row says
/// otherwise: the provider's `metadata.agent.profile`, else the profile
/// implied by its wire protocol.
fn provider_agent_profile(provider: &CatalogProvider) -> AgentProfileKind {
    agent_metadata(provider.metadata())
        .profile
        .unwrap_or_else(|| match provider.adapter().as_str() {
            "anthropic" | "bedrock" => AgentProfileKind::Anthropic,
            "gemini" => AgentProfileKind::Gemini,
            _ => AgentProfileKind::OpenAi,
        })
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
pub fn enabled_providers(catalog: &Catalog) -> Vec<&CatalogProvider> {
    let mut providers: Vec<_> = catalog
        .providers()
        .filter(|provider| provider.is_enabled())
        .collect();
    providers.sort_by(|left, right| {
        right
            .priority()
            .cmp(&left.priority())
            .then_with(|| left.id().cmp(right.id()))
    });
    providers
}

/// Enabled providers that Fabro lists to operators. Stand-in providers such
/// as `openai-codex` route requests but are not offerings of their own.
#[must_use]
pub fn listed_providers(catalog: &Catalog) -> Vec<&CatalogProvider> {
    enabled_providers(catalog)
        .into_iter()
        .filter(|provider| provider.stands_in_for().is_none())
        .collect()
}

/// The ids of every enabled provider.
#[must_use]
pub fn enabled_provider_ids(catalog: &Catalog) -> HashSet<ProviderId> {
    enabled_providers(catalog)
        .into_iter()
        .map(|provider| provider.id().clone())
        .collect()
}

/// Looks up an enabled provider by id or alias.
#[must_use]
pub fn provider<'a>(catalog: &'a Catalog, selector: &str) -> Option<&'a CatalogProvider> {
    catalog
        .provider(selector)
        .ok()
        .filter(|provider| provider.is_enabled())
}

/// Canonicalizes a provider id or alias to its catalog id, when enabled.
#[must_use]
pub fn canonical_provider_id(catalog: &Catalog, selector: &str) -> Option<ProviderId> {
    provider(catalog, selector).map(|provider| provider.id().clone())
}

/// The models of a provider, in catalog order.
#[must_use]
pub fn provider_models(provider: &CatalogProvider) -> Vec<ModelEntry<'_>> {
    provider
        .models()
        .map(|model| ModelEntry { provider, model })
        .collect()
}

/// Every model across listed providers, provider priority order.
#[must_use]
pub fn models(catalog: &Catalog) -> Vec<ModelEntry<'_>> {
    listed_providers(catalog)
        .into_iter()
        .flat_map(provider_models)
        .collect()
}

/// Finds a model on an enabled provider by id, alias, or wire id.
#[must_use]
pub fn model_on_provider<'a>(
    catalog: &'a Catalog,
    provider_selector: &str,
    model_selector: &str,
) -> Option<ModelEntry<'a>> {
    let provider = provider(catalog, provider_selector)?;
    // lithos matches ids and aliases. The provider's wire id (an aggregator's
    // `vendor/model`) is accepted too, so a selector copied from the
    // provider's own listing lands on the catalog row instead of passing
    // through unknown.
    let model = provider.model(model_selector).or_else(|| {
        provider
            .models()
            .find(|model| model.api_model() == model_selector)
    })?;
    Some(ModelEntry { provider, model })
}

/// Models matching `selector` by id or alias, ordered like lithos selection:
/// exact ids before aliases, then provider priority.
#[must_use]
pub fn models_matching<'a>(catalog: &'a Catalog, selector: &str) -> Vec<ModelEntry<'a>> {
    let mut matches: Vec<_> = enabled_providers(catalog)
        .into_iter()
        .flat_map(provider_models)
        .filter(|entry| {
            entry.model.id().as_str() == selector
                || entry.model.aliases().iter().any(|alias| alias == selector)
        })
        .collect();
    matches.sort_by_key(|entry| entry.model.id().as_str() != selector);
    matches
}

/// Whether `selector` names a model on any enabled provider.
#[must_use]
pub fn is_model_selector(catalog: &Catalog, selector: &str) -> bool {
    !models_matching(catalog, selector).is_empty()
}

/// Whether `selector` names an enabled provider.
#[must_use]
pub fn is_provider_selector(catalog: &Catalog, selector: &str) -> bool {
    provider(catalog, selector).is_some()
}

/// The default model of an enabled provider.
#[must_use]
pub fn default_model<'a>(catalog: &'a Catalog, provider_selector: &str) -> Option<ModelEntry<'a>> {
    let provider = provider(catalog, provider_selector)?;
    let default = provider.default_model()?;
    model_on_provider(catalog, provider.id().as_str(), default)
}

/// The model Fabro probes a provider with: the `probe` model, else the
/// provider default.
#[must_use]
pub fn probe_model<'a>(catalog: &'a Catalog, provider_selector: &str) -> Option<ModelEntry<'a>> {
    let provider = provider(catalog, provider_selector)?;
    provider_models(provider)
        .into_iter()
        .find(|entry| entry.model.is_probe())
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
        .filter(|provider| ready.contains(provider.id()))
        .chain(providers.iter())
        .find_map(|provider| default_model(catalog, provider.id().as_str()))
}

/// The small utility model across `ready` providers: the first
/// `small_default` model in provider priority order, else the ready default.
#[must_use]
pub fn small_default_for_ready<'a>(
    catalog: &'a Catalog,
    ready: &HashSet<ProviderId>,
) -> Option<ModelEntry<'a>> {
    enabled_providers(catalog)
        .into_iter()
        .filter(|provider| ready.contains(provider.id()))
        .flat_map(provider_models)
        .find(|entry| entry.model.is_small_default())
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
    let provider = provider(catalog, provider_selector)?;
    let model = model_selector.and_then(|selector| provider.model(selector));
    Some(match model {
        Some(model) => ModelEntry { provider, model }.agent_profile(),
        None => provider_agent_profile(provider),
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
    provider_models(target)
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
        .map(|provider| {
            (
                provider.id().clone(),
                provider_models(provider)
                    .into_iter()
                    .map(|entry| entry.model.id().clone())
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
    fn builtins_ship_fabro_defaults() {
        let catalog = test_catalog();
        let ids: Vec<_> = enabled_providers(&catalog)
            .iter()
            .map(|provider| provider.id().to_string())
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
                .any(|provider| provider.id().as_str() == "openai-codex"),
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
            "overlay values win over the built-ins"
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
    fn probe_and_small_default_follow_the_catalog() {
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
            Some(AgentProfileKind::Kimi),
            "a passthrough model on Moonshot takes the provider's Kimi profile"
        );
        assert_eq!(
            agent_profile(&catalog, "deepseek", None),
            Some(AgentProfileKind::OpenAi)
        );
        assert_eq!(
            agent_profile(&catalog, "openrouter", None),
            None,
            "disabled providers have no profile to offer"
        );
        assert_eq!(
            agent_profile(&catalog, "moonshot", Some("kimi-k3")),
            Some(AgentProfileKind::Kimi)
        );
        assert_eq!(
            agent_profile(&catalog, "openai", Some("gpt-6-astra")),
            Some(AgentProfileKind::Gpt6)
        );
        assert_eq!(
            agent_profile(&catalog, "anthropic", Some("claude-sonnet-4.5")),
            Some(AgentProfileKind::Anthropic)
        );
    }

    #[test]
    fn reasoning_by_default_reads_agent_metadata_then_capabilities() {
        let catalog = test_catalog();
        let kimi = model_on_provider(&catalog, "moonshot", "kimi-k2.5").unwrap();
        assert!(kimi.reasons_by_default(), "the catalog row says so");
        let sonnet = model_on_provider(&catalog, "anthropic", "claude-sonnet-4.5").unwrap();
        assert!(
            !sonnet.reasons_by_default(),
            "a thinking-budget model reasons only when asked"
        );
    }
}
