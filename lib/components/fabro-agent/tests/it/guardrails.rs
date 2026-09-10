use std::sync::Arc;

use fabro_agent::{AgentProfile, AgentProfileBuilder};
use fabro_llm::catalog;
use fabro_llm::test_support::test_catalog;

#[test]
fn profile_context_window_matches_catalog_for_default_models() {
    let catalog = Arc::new(test_catalog());
    for provider in catalog::listed_providers(&catalog) {
        let provider_id = provider.id().clone();
        let Some(default) = catalog::default_model(&catalog, provider_id.as_str()) else {
            // Deployment-defined providers (LiteLLM, Modal, Ollama) carry no
            // built-in default model.
            continue;
        };
        let model = default.model.id().clone();
        let context_window = default.model.limits().map_or_else(
            || panic!("no limits for {provider_id}/{model} in catalog"),
            |limits| usize::try_from(limits.context_tokens).expect("context fits usize"),
        );

        let profile: Box<dyn AgentProfile> = AgentProfileBuilder::new(
            default.agent_profile(),
            provider_id.clone(),
            model.as_str(),
            Arc::clone(&catalog),
        )
        .build();

        assert_eq!(
            profile.context_window_size(),
            context_window,
            "context_window_size mismatch for {provider_id} model '{model}': profile={} catalog={}",
            profile.context_window_size(),
            context_window
        );
    }
}
