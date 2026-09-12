//! The one failure-classification rule that is Fabro's own.
//!
//! Retry, auth, cancellation, and failover questions are answered by the
//! lithos `Error` and `ErrorData` themselves. What stays here is the loop and
//! restart detector's signature format, which names Fabro's own categories.

use std::time::Duration;

use lithos_llm::catalog::ProviderId;
use lithos_llm::types::{ErrorData, ErrorKind};

use crate::gateway::reset_prose_is_naive;

/// A rate-limit wait longer than the retry middleware's `Retry-After` cap.
///
/// The retry layer honors provider waits only up to this bound; anything
/// longer cannot be bridged by count-based retries and must be surfaced to
/// the caller (the workflow parks such runs as a soft stop, fabro-a3d8).
pub const LONG_RATE_LIMIT_WINDOW: Duration = Duration::from_mins(1);

/// The provider-advised reset window when it is a long one.
///
/// `Some(window)` only for a rate-limit failure whose advised wait exceeds
/// [`LONG_RATE_LIMIT_WINDOW`] — the multi-hour usage-window case. Short
/// 429s return `None` and keep today's retry behavior.
#[must_use]
pub fn long_rate_limit_window(error: &ErrorData) -> Option<Duration> {
    if error.kind() != ErrorKind::RateLimit {
        return None;
    }
    let window = error.provider_retry_after()?;
    (window > LONG_RATE_LIMIT_WINDOW).then_some(window)
}

/// Whether a rate-limit failure announced a reset whose deadline carries no
/// UTC offset (fabro-0607).
///
/// The real wait is unknown — the provider's wallclock offset is missing
/// (zai sends Beijing time), so no duration may be computed from it. Such
/// errors must still park the run (soft stop): the usage window is closed
/// for an unknown, potentially multi-hour span, and count-based retries
/// would burn attempts against a live limit.
#[must_use]
pub fn rate_limit_window_unknown(error: &ErrorData) -> bool {
    if error.kind() != ErrorKind::RateLimit {
        return false;
    }
    // A trusted value — a Retry-After header or an offset-carrying
    // timestamp — governs when present.
    if error.provider_retry_after().is_some() {
        return false;
    }
    reset_prose_is_naive(error.message())
}

/// A stable `category|provider|detail` string for loop and restart detection.
///
/// The category is `api_canceled` for a cancelled call, `api_transient` for a
/// failure the provider may be asked to repeat, and `api_deterministic` for
/// everything else; the detail is the error kind's stored spelling.
#[must_use]
pub fn failure_signature_hint(error: &ErrorData) -> String {
    let provider = error.provider().map_or("unknown", ProviderId::as_str);
    let category = if error.is_cancelled() {
        "api_canceled"
    } else if error.is_retryable() {
        "api_transient"
    } else {
        "api_deterministic"
    };
    let kind: ErrorKind = error.kind();
    format!("{category}|{provider}|{}", kind.as_str())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use lithos_llm::types::{Error, RetryClassification};

    use super::*;

    #[test]
    fn long_windows_are_only_rate_limits_beyond_the_cap() {
        let error = |kind, millis| {
            ErrorData::from(
                Error::new(kind, "boom")
                    .with_provider(ProviderId::new("zai"))
                    .with_retry(RetryClassification::Safe)
                    .with_provider_retry_after(Duration::from_millis(millis)),
            )
        };
        assert_eq!(
            long_rate_limit_window(&error(ErrorKind::RateLimit, 5 * 3_600_000)),
            Some(Duration::from_hours(5))
        );
        // A short 429 advises a wait the retry layer can honor.
        assert_eq!(
            long_rate_limit_window(&error(ErrorKind::RateLimit, 2_000)),
            None
        );
        // No advised wait at all, or a long wait on another kind.
        let none = ErrorData::from(
            Error::new(ErrorKind::RateLimit, "boom")
                .with_provider(ProviderId::new("zai"))
                .with_retry(RetryClassification::Safe),
        );
        assert_eq!(long_rate_limit_window(&none), None);
        assert_eq!(
            long_rate_limit_window(&error(ErrorKind::Server, 5 * 3_600_000)),
            None
        );
    }

    fn error(kind: ErrorKind) -> ErrorData {
        Error::new(kind, "boom")
            .with_provider(ProviderId::new("openai"))
            .data()
    }

    #[test]
    fn signatures_name_category_provider_and_kind() {
        assert_eq!(
            failure_signature_hint(&error(ErrorKind::InvalidRequest)),
            "api_deterministic|openai|invalid_request"
        );
        assert_eq!(
            failure_signature_hint(
                &Error::new(ErrorKind::RateLimit, "boom")
                    .with_provider(ProviderId::new("openai"))
                    .with_retry(RetryClassification::Safe)
                    .data()
            ),
            "api_transient|openai|rate_limit"
        );
        assert_eq!(
            failure_signature_hint(&error(ErrorKind::Cancelled)),
            "api_canceled|openai|cancelled"
        );
    }
}
