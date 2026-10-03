use serde::{Deserialize, Serialize};

use crate::{FailureDetail, FailureReason};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunFailure {
    pub reason: FailureReason,
    pub detail: FailureDetail,
}

/// Fork (fabro-986b): classify a failure as quota-rate-limit class. The
/// engine-side quota park (ADR-0021) writes `SoftStop` + transient-infra +
/// a `rate_limit` signature; server-side gates and the automation breaker
/// exemption key off this predicate.
pub fn is_quota_rate_limit_failure(failure: &RunFailure) -> bool {
    use crate::outcome::FailureCategory;

    failure.reason == FailureReason::SoftStop
        && failure.detail.category == FailureCategory::TransientInfra
        && failure
            .detail
            .signature
            .as_ref()
            .is_some_and(|signature| signature.as_str().contains("rate_limit"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::{FailureCategory, FailureDetail, FailureReason, RunFailure};

    #[test]
    fn run_failure_wraps_failure_detail_with_reason() {
        let detail = FailureDetail::new("ACP turn failed", FailureCategory::Deterministic);
        let failure = RunFailure {
            reason: FailureReason::WorkflowError,
            detail,
        };

        let value = serde_json::to_value(&failure).expect("run failure should serialize");

        assert_eq!(value["reason"], "workflow_error");
        assert_eq!(value["detail"]["message"], "ACP turn failed");
        assert_eq!(value["detail"]["category"], "deterministic");
        assert_eq!(
            value,
            json!({
                "reason": "workflow_error",
                "detail": {
                    "message": "ACP turn failed",
                    "category": "deterministic"
                }
            })
        );
    }
}
