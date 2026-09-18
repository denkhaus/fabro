use serde::{Deserialize, Serialize};

use crate::{FailureDetail, FailureReason};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunFailure {
    pub reason: FailureReason,
    pub detail: FailureDetail,
}

/// Whether a terminal run failure is a quota/rate-limit park
/// (`api_transient|<provider>|rate_limit` signature family, fabro-e566).
///
/// Structural detection only — SoftStop reason + TransientInfra category +
/// a `rate_limit`-bearing signature (`rate_limit` or `rate_limited`
/// depending on provider); message prose is never classified here. This is
/// the ONE classifier: the lifecycle table remaps such `run.failed` events
/// to `Blocked { quota_rate_limit }`, and fabro-server's `is_quota_park`
/// (fork line recovery / breaker exemption) delegates here.
#[must_use]
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

    use super::super::run_failure::is_quota_rate_limit_failure;
    use crate::{FailureCategory, FailureDetail, FailureReason, FailureSignature, RunFailure};

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

    fn quota_failure() -> RunFailure {
        let mut detail = FailureDetail::new(
            "LLM error: provider zai Usage limit reached for 5 hour. Your limit will reset at \
             2026-09-18 16:56:19",
            FailureCategory::TransientInfra,
        );
        detail.signature = Some(FailureSignature("api_transient|zai|rate_limit".to_string()));
        RunFailure {
            reason: FailureReason::SoftStop,
            detail,
        }
    }

    #[test]
    fn quota_rate_limit_signature_classifies_as_quota_park() {
        assert!(is_quota_rate_limit_failure(&quota_failure()));
    }

    #[test]
    fn non_quota_failures_do_not_classify_as_quota_parks() {
        // Soft stop without the rate_limit signature (non-quota soft stop).
        let mut no_signature = quota_failure();
        no_signature.detail.signature = None;
        assert!(!is_quota_rate_limit_failure(&no_signature));
        // Rate-limit signature on a deterministic category is not a park.
        let mut wrong_category = quota_failure();
        wrong_category.detail.category = FailureCategory::Deterministic;
        assert!(!is_quota_rate_limit_failure(&wrong_category));
        // A workflow error never parks, signature notwithstanding.
        let mut hard = quota_failure();
        hard.reason = FailureReason::WorkflowError;
        assert!(!is_quota_rate_limit_failure(&hard));
    }
}
