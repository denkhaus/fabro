//! The model and usage facts the records carry, as the view names them.

use fabro_types::ModelRef;
use lithos_llm::catalog::{ModelId, ProviderId};
use lithos_llm::types::Usage;
use serde_json::Value;

pub(super) fn usage_of(value: Option<&Value>) -> Option<Usage> {
    serde_json::from_value(value?.clone()).ok()
}

/// `provider/model` into its parts, or the model alone.
pub(super) fn split_model(model: &str) -> (Option<&str>, &str) {
    match model.split_once('/') {
        Some((provider, model)) if !provider.is_empty() && !model.is_empty() => {
            (Some(provider), model)
        }
        _ => (None, model),
    }
}

pub(super) fn model_ref(provider: Option<&str>, model: &str) -> Option<ModelRef> {
    let provider = provider.filter(|provider| !provider.is_empty())?;
    Some(ModelRef::new(
        ProviderId::new(provider),
        ModelId::new(model),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_selector_splits_into_provider_and_model() {
        assert_eq!(split_model("openai/gpt-5.4"), (Some("openai"), "gpt-5.4"));
        assert_eq!(split_model("gpt-5.4"), (None, "gpt-5.4"));
        assert!(model_ref(None, "gpt-5.4").is_none());
        assert!(model_ref(Some("openai"), "gpt-5.4").is_some());
    }
}
