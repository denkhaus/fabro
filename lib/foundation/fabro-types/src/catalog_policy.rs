//! Fabro's `metadata.fabro` catalog namespace.
//!
//! lithos-llm owns provider and model facts. Fabro attaches its own policy to
//! each entry under `metadata.fabro`, which lithos carries verbatim and never
//! interprets. These types are the typed view of that namespace. Every field
//! is optional in the TOML; the accessors here apply Fabro's defaults.

use lithos_llm::catalog::{CatalogModel, CatalogProvider};
use serde::{Deserialize, Serialize};

use crate::AgentProfileKind;

/// Name of the metadata namespace Fabro owns on catalog entries.
pub const FABRO_METADATA_NAMESPACE: &str = "fabro";

/// Provider-level Fabro policy.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProviderPolicy {
    /// Whether Fabro offers this provider at all. Missing means enabled.
    pub enabled:       Option<bool>,
    /// Default agent profile for models on this provider.
    pub agent_profile: Option<AgentProfileKind>,
    /// Where an operator obtains an API key.
    pub api_key_url:   Option<String>,
    /// Ordered credential references (`env:NAME`, `vault:NAME`, `aws_sigv4`).
    /// The first that resolves wins.
    pub credentials:   Vec<String>,
    /// Extra request headers. Values are literal text or `{{ secrets.NAME }}`
    /// interpolation strings resolved against the vault.
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub extra_headers: std::collections::BTreeMap<String, String>,
    /// Another provider this one serves requests for when that provider has
    /// no credentials of its own. Used by `openai-codex`, which answers
    /// `openai` requests with a ChatGPT OAuth credential.
    pub stands_in_for: Option<String>,
}

impl ProviderPolicy {
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
}

/// Model-level Fabro policy.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelPolicy {
    /// Whether Fabro offers this model. Missing means enabled.
    pub enabled:              Option<bool>,
    /// Agent profile override for this model.
    pub agent_profile:        Option<AgentProfileKind>,
    /// Model family label for display and grouping.
    pub family:               Option<String>,
    /// Training data cutoff label.
    pub training:             Option<String>,
    /// Public knowledge cutoff label.
    pub knowledge_cutoff:     Option<String>,
    /// Estimated output tokens per second.
    pub estimated_output_tps: Option<f64>,
    /// Preferred for small utility calls such as title generation.
    pub small_default:        bool,
    /// Preferred for provider connectivity probes.
    pub probe:                bool,
    /// Whether requests reason when no effort is requested. Missing means
    /// "reasons when the model supports reasoning".
    pub reasoning_by_default: Option<bool>,
}

impl ModelPolicy {
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }
}

/// Reads a provider's Fabro policy. Malformed metadata falls back to the
/// defaults; the catalog build is the place to validate shape, and Fabro's
/// own policy file is checked in tests.
#[must_use]
pub fn provider_policy(provider: &CatalogProvider) -> ProviderPolicy {
    provider
        .metadata()
        .namespace::<ProviderPolicy>(FABRO_METADATA_NAMESPACE)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// Reads a model's Fabro policy.
#[must_use]
pub fn model_policy(model: &CatalogModel) -> ModelPolicy {
    model
        .metadata()
        .namespace::<ModelPolicy>(FABRO_METADATA_NAMESPACE)
        .ok()
        .flatten()
        .unwrap_or_default()
}

/// The agent profile a model runs under: the model override, then the
/// provider default, then the profile implied by the provider's adapter.
#[must_use]
pub fn effective_agent_profile(
    provider: &CatalogProvider,
    model: &CatalogModel,
) -> AgentProfileKind {
    model_policy(model)
        .agent_profile
        .or(provider_policy(provider).agent_profile)
        .unwrap_or_else(|| default_agent_profile(provider))
}

/// The agent profile implied by a provider's wire protocol.
#[must_use]
pub fn default_agent_profile(provider: &CatalogProvider) -> AgentProfileKind {
    match provider.adapter().as_str() {
        "anthropic" | "bedrock" => AgentProfileKind::Anthropic,
        "gemini" => AgentProfileKind::Gemini,
        _ => AgentProfileKind::OpenAi,
    }
}

#[cfg(test)]
mod tests {
    use lithos_llm::catalog::Catalog;

    use super::*;

    fn catalog() -> Catalog {
        Catalog::builder()
            .toml_layer(
                "test",
                r#"
schema_version = 1

[providers.acme]
display_name = "Acme"
adapter = "openai-compatible"
codec = "openai-chat"
base_url = "https://acme.test/v1"
auth = { type = "bearer" }
default_model = "large"

[providers.acme.metadata.fabro]
enabled = false
credentials = ["env:ACME_API_KEY"]
agent_profile = "kimi"

[providers.acme.models.large]
display_name = "Large"
api_model = "large"

[providers.acme.models.large.metadata.fabro]
small_default = true
probe = true
family = "acme"

[providers.acme.models.small]
display_name = "Small"
api_model = "small"
[providers.acme.models.small.metadata.fabro]
agent_profile = "openai"
enabled = false
"#,
            )
            .unwrap()
            .build()
            .unwrap()
    }

    #[test]
    fn reads_provider_and_model_policy() {
        let catalog = catalog();
        let provider = catalog.provider("acme").unwrap();
        let policy = provider_policy(provider);
        assert!(!policy.is_enabled());
        assert_eq!(policy.credentials, vec!["env:ACME_API_KEY"]);
        assert_eq!(policy.agent_profile, Some(AgentProfileKind::Kimi));

        let large = provider.model("large").unwrap();
        let policy = model_policy(large);
        assert!(policy.small_default && policy.probe && policy.is_enabled());
        assert_eq!(policy.family.as_deref(), Some("acme"));
        assert_eq!(
            effective_agent_profile(provider, large),
            AgentProfileKind::Kimi
        );

        let small = provider.model("small").unwrap();
        assert!(!model_policy(small).is_enabled());
        assert_eq!(
            effective_agent_profile(provider, small),
            AgentProfileKind::OpenAi
        );
    }

    #[test]
    fn missing_namespace_yields_defaults() {
        let catalog = Catalog::builder()
            .toml_layer(
                "test",
                r#"
schema_version = 1
[providers.bare]
display_name = "Bare"
adapter = "anthropic"
codec = "anthropic-messages"
base_url = "https://bare.test"
auth = { type = "none" }
[providers.bare.models.m]
display_name = "M"
api_model = "m"
"#,
            )
            .unwrap()
            .build()
            .unwrap();
        let provider = catalog.provider("bare").unwrap();
        assert!(provider_policy(provider).is_enabled());
        let model = provider.model("m").unwrap();
        assert!(model_policy(model).is_enabled());
        assert_eq!(
            effective_agent_profile(provider, model),
            AgentProfileKind::Anthropic
        );
    }
}
