pub use fabro_core::outcome::{
    FailureCategory, FailureDetail, OutcomeMeta, StageOutcome, StageState,
};
use fabro_llm::lithos_catalog::Catalog;
use fabro_types::ModelRef;
pub use fabro_types::ModelUsage;
use lithos_llm::types::{Cost, TokenCounts, Usage};

use crate::error::{Error, FailureSignature, classify_failure_reason};

pub type Outcome = fabro_core::Outcome<Option<ModelUsage>>;

/// Prices `tokens` on `model` from the catalog: the usage carries a
/// [`CostSource::Catalog`](lithos_llm::types::CostSource::Catalog) cost when
/// the catalog has rates for the model, and no cost otherwise.
///
/// The provider must be one the catalog knows; a passthrough model on a known
/// provider is priced with no cost, since the catalog has no rates for it.
pub fn model_usage_from_llm(
    catalog: &Catalog,
    model: &ModelRef,
    tokens: TokenCounts,
) -> Result<ModelUsage, Error> {
    if catalog.enabled_provider(model.provider.as_str()).is_none() {
        return Err(Error::Precondition(format!(
            "Provider \"{}\" is not configured",
            model.provider
        )));
    }
    let cost = catalog.estimate_cost(&model.handle(), tokens, model.speed);
    Ok(ModelUsage::new(model.clone(), Usage { tokens, cost }))
}

/// `usage` with `cost` in place of whatever it carried, when a provider
/// reported one; `None` keeps the usage as it is.
#[must_use]
pub fn with_reported_cost(mut usage: ModelUsage, cost: Option<Cost>) -> ModelUsage {
    if let Some(cost) = cost {
        usage.usage.cost = Some(cost);
    }
    usage
}

pub trait OutcomeExt: Sized {
    fn fail_deterministic(reason: impl Into<String>) -> Self;
    fn fail_classify(reason: impl Into<String>) -> Self;
    fn retry_classify(reason: impl Into<String>) -> Self;
    fn simulated(node_id: &str) -> Self;
    #[must_use]
    fn with_signature(self, sig: Option<impl Into<String>>) -> Self;
    fn failure_reason(&self) -> Option<&str>;
    fn failure_category(&self) -> Option<FailureCategory>;
    fn classified_failure_category(&self) -> Option<FailureCategory>;
}

impl OutcomeExt for Outcome {
    fn fail_deterministic(reason: impl Into<String>) -> Self {
        Self {
            status: StageOutcome::Failed {
                retry_requested: false,
            },
            failure: Some(FailureDetail::new(reason, FailureCategory::Deterministic)),
            ..Self::default()
        }
    }

    fn fail_classify(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        let category = classify_failure_reason(&reason);
        Self {
            status: StageOutcome::Failed {
                retry_requested: false,
            },
            failure: Some(FailureDetail::new(reason, category)),
            ..Self::default()
        }
    }

    fn retry_classify(reason: impl Into<String>) -> Self {
        let reason = reason.into();
        let category = classify_failure_reason(&reason);
        Self {
            status: StageOutcome::Failed {
                retry_requested: true,
            },
            failure: Some(FailureDetail::new(reason, category)),
            ..Self::default()
        }
    }

    fn simulated(node_id: &str) -> Self {
        Self {
            notes: Some(format!("[Simulated] {node_id}")),
            ..Self::success()
        }
    }

    fn with_signature(mut self, sig: Option<impl Into<String>>) -> Self {
        if let Some(ref mut failure) = self.failure {
            failure.signature = sig.map(|sig| FailureSignature(sig.into()));
        }
        self
    }

    fn failure_reason(&self) -> Option<&str> {
        self.failure
            .as_ref()
            .map(|failure| failure.message.as_str())
    }

    fn failure_category(&self) -> Option<FailureCategory> {
        self.failure.as_ref().map(|failure| failure.category)
    }

    fn classified_failure_category(&self) -> Option<FailureCategory> {
        match self.status {
            StageOutcome::Succeeded | StageOutcome::PartiallySucceeded | StageOutcome::Skipped => {
                None
            }
            StageOutcome::Failed { .. } => self
                .failure_category()
                .or(Some(FailureCategory::Deterministic)),
        }
    }
}

#[must_use]
pub fn format_cost(cost: f64) -> String {
    format!("${cost:.2}")
}

#[cfg(test)]
mod tests {
    use fabro_llm::lithos_catalog::Catalog;
    use fabro_llm::test_support::{test_catalog, test_catalog_with_overlay};
    use fabro_types::ModelRef;
    use lithos_llm::catalog::{ModelId, ProviderId, builtin};
    use lithos_llm::types::{Cost, CostSource, Speed, TokenCounts};

    use super::{OutcomeExt, model_usage_from_llm, with_reported_cost};

    fn model_ref(provider: ProviderId, model_id: &str, speed: Option<Speed>) -> ModelRef {
        ModelRef::new(provider, ModelId::new(model_id)).with_speed(speed)
    }

    fn catalog() -> Catalog {
        test_catalog()
    }

    #[test]
    fn model_usage_from_llm_prices_openai_cached_input_and_reasoning_output() {
        // Stay under the 272k long-context tier so the standard rates apply.
        let usage = TokenCounts {
            input: 100_000,
            output: 25_000,
            reasoning: 5_000,
            cache_read: 50_000,
            ..TokenCounts::default()
        };
        let billed = model_usage_from_llm(
            &catalog(),
            &model_ref(builtin::openai(), "gpt-5.4", None),
            usage,
        )
        .unwrap();

        // 100k input at $2.50/M + 50k cached at $0.25/M + 30k output at $15/M.
        assert_eq!(
            billed.usage.cost,
            Some(Cost {
                usd_micros: 712_500,
                source:     CostSource::Catalog,
            })
        );
        assert_eq!(billed.usage.tokens.output, 25_000);
        assert_eq!(billed.usage.tokens.reasoning, 5_000);
    }

    #[test]
    fn response_cost_overrides_catalog_estimate() {
        let usage = TokenCounts {
            input: 11,
            output: 7,
            ..TokenCounts::default()
        };
        let reported = Cost {
            usd_micros: 125_000,
            source:     CostSource::Provider,
        };
        let billed = with_reported_cost(
            model_usage_from_llm(
                &catalog(),
                &model_ref(builtin::openai(), "gpt-5.4", None),
                usage,
            )
            .unwrap(),
            Some(reported),
        );

        assert_eq!(billed.usage.cost, Some(reported));
        assert_eq!(billed.usage.tokens, usage);
    }

    #[test]
    fn retry_classify_marks_failed_outcome_with_retry_request() {
        let outcome = crate::outcome::Outcome::retry_classify("timeout");

        assert_eq!(outcome.status, crate::outcome::StageOutcome::Failed {
            retry_requested: true,
        });
        assert!(outcome.status.retry_requested());
    }

    #[test]
    fn model_usage_from_llm_prices_anthropic_fast_mode_cache_write_rates() {
        let usage = TokenCounts {
            input:       100_000,
            output:      10_000,
            reasoning:   5_000,
            cache_read:  20_000,
            cache_write: 30_000,
        };
        let billed = model_usage_from_llm(
            &catalog(),
            &model_ref(builtin::anthropic(), "claude-opus-5", Some(Speed::Fast)),
            usage,
        )
        .unwrap();

        // Fast rates: $10/M input, $50/M output (incl. reasoning), $1/M cache
        // read, $12.50/M cache write.
        assert_eq!(
            billed.usage.cost.map(|cost| cost.usd_micros),
            Some(2_145_000)
        );
    }

    #[test]
    fn model_usage_from_llm_uses_injected_custom_catalog() {
        let catalog = test_catalog_with_overlay(
            r#"
[providers.proxy]
display_name = "Proxy"
adapter = "openai-compatible"
codec = "openai-chat"
base_url = "https://proxy.example/v1"
auth = { type = "bearer" }
default_model = "canonical-model"

[providers.proxy.models.canonical-model]
display_name = "Canonical Model"
api_model = "wire-model"
limits = { context_tokens = 1000, max_output_tokens = 500 }
capabilities = { text = true, tools = true }
pricing = { input_usd_micros_per_million = 1000000, output_usd_micros_per_million = 2000000 }
"#,
        );
        let usage = TokenCounts {
            input: 1_000_000,
            output: 500_000,
            ..TokenCounts::default()
        };
        let billed = model_usage_from_llm(
            &catalog,
            &model_ref(ProviderId::new("proxy"), "canonical-model", None),
            usage,
        )
        .unwrap();

        assert_eq!(
            billed.usage.cost.map(|cost| cost.usd_micros),
            Some(2_000_000)
        );
        assert_eq!(billed.model_id(), "canonical-model");
    }

    #[test]
    fn passthrough_model_on_known_provider_has_no_cost() {
        let billed = model_usage_from_llm(
            &catalog(),
            &model_ref(builtin::openai(), "brand-new-model", None),
            TokenCounts {
                input: 10,
                output: 5,
                ..TokenCounts::default()
            },
        )
        .unwrap();
        assert_eq!(billed.usage.cost, None);
        assert_eq!(billed.usage.tokens.input, 10);
    }

    #[test]
    fn unknown_provider_is_a_precondition_failure() {
        let error = model_usage_from_llm(
            &catalog(),
            &model_ref(ProviderId::new("nowhere"), "model", None),
            TokenCounts::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("not configured"), "{error}");
    }
}
