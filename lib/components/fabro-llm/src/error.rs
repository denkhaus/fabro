//! Classification of lithos errors for Fabro's retry, failover, and failure
//! signature policies, plus the stored form of a failure.
//!
//! lithos's live [`Error`] carries a source chain and is therefore neither
//! `Clone` nor serializable. Fabro records failures in events and agent
//! errors, so it works with [`LlmError`], a thin wrapper over lithos's own
//! [`ErrorData`] projection. Every policy here reads through [`ErrorFacts`]
//! and so applies to both forms.

use std::fmt;
use std::time::Duration;

use fabro_types::ProviderId;
use lithos_llm::types::{Error, ErrorData, ErrorKind, RetryClassification};
use serde::{Deserialize, Serialize};

/// The facts Fabro's policies read from an LLM failure.
pub trait ErrorFacts {
    fn kind(&self) -> ErrorKind;
    fn message(&self) -> &str;
    fn provider(&self) -> Option<&ProviderId>;
    fn provider_code(&self) -> Option<&str>;
    fn status(&self) -> Option<u16>;
    fn retry_classification(&self) -> RetryClassification;

    /// The delay the classification advises, when repeating is safe after
    /// a wait.
    fn retry_after(&self) -> Option<Duration> {
        self.retry_classification().delay()
    }
}

impl ErrorFacts for Error {
    fn kind(&self) -> ErrorKind {
        Self::kind(self)
    }

    fn message(&self) -> &str {
        Self::message(self)
    }

    fn provider(&self) -> Option<&ProviderId> {
        Self::provider(self)
    }

    fn provider_code(&self) -> Option<&str> {
        Self::provider_code(self)
    }

    fn status(&self) -> Option<u16> {
        Self::status(self)
    }

    fn retry_classification(&self) -> RetryClassification {
        Self::retry_classification(self)
    }
}

impl ErrorFacts for ErrorData {
    fn kind(&self) -> ErrorKind {
        self.kind.clone()
    }

    fn message(&self) -> &str {
        &self.message
    }

    fn provider(&self) -> Option<&ProviderId> {
        self.provider.as_ref()
    }

    fn provider_code(&self) -> Option<&str> {
        self.provider_code.as_deref()
    }

    fn status(&self) -> Option<u16> {
        self.status
    }

    fn retry_classification(&self) -> RetryClassification {
        self.retry
    }
}

/// A cloneable, serializable LLM failure.
///
/// This is lithos's [`ErrorData`] projection with Fabro's policy helpers
/// attached. It is what agent errors, run events, and API responses carry;
/// the live [`Error`] converts into it at the boundary where a failure stops
/// being handled and starts being recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LlmError(Box<ErrorData>);

impl LlmError {
    /// A failure Fabro itself raises, never retried.
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self::from(Error::new(kind, message))
    }

    #[must_use]
    pub fn data(&self) -> &ErrorData {
        &self.0
    }

    #[must_use]
    pub fn into_data(self) -> ErrorData {
        *self.0
    }

    /// The immediate source of the failure, rendered as text.
    #[must_use]
    pub fn source_message(&self) -> Option<&str> {
        self.0.source_message.as_deref()
    }

    /// The provider's advised wait, whatever the error kind.
    #[must_use]
    pub fn provider_retry_after(&self) -> Option<Duration> {
        self.0
            .provider_retry_after_millis
            .map(Duration::from_millis)
    }

    #[must_use]
    pub fn is_retryable(&self) -> bool {
        is_retryable(self)
    }

    #[must_use]
    pub fn is_auth_error(&self) -> bool {
        is_auth_error(self)
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        is_cancelled(self)
    }

    #[must_use]
    pub fn failover_eligible(&self) -> bool {
        failover_eligible(self)
    }

    #[must_use]
    pub fn failure_signature_hint(&self) -> String {
        failure_signature_hint(self)
    }
}

impl ErrorFacts for LlmError {
    fn kind(&self) -> ErrorKind {
        self.0.kind.clone()
    }

    fn message(&self) -> &str {
        &self.0.message
    }

    fn provider(&self) -> Option<&ProviderId> {
        self.0.provider.as_ref()
    }

    fn provider_code(&self) -> Option<&str> {
        self.0.provider_code.as_deref()
    }

    fn status(&self) -> Option<u16> {
        self.0.status
    }

    fn retry_classification(&self) -> RetryClassification {
        self.0.retry
    }
}

impl fmt::Display for LlmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0.message)
    }
}

impl std::error::Error for LlmError {}

impl From<Error> for LlmError {
    fn from(error: Error) -> Self {
        Self(Box::new(error.data()))
    }
}

impl From<&Error> for LlmError {
    fn from(error: &Error) -> Self {
        Self(Box::new(error.data()))
    }
}

impl From<ErrorData> for LlmError {
    fn from(data: ErrorData) -> Self {
        Self(Box::new(data))
    }
}

/// Whether repeating the same call on the same provider may succeed.
#[must_use]
pub fn is_retryable<E: ErrorFacts + ?Sized>(error: &E) -> bool {
    !matches!(error.retry_classification(), RetryClassification::Never)
}

/// Whether the failure came from a credential problem.
#[must_use]
pub fn is_auth_error<E: ErrorFacts + ?Sized>(error: &E) -> bool {
    matches!(
        error.kind(),
        ErrorKind::Authentication | ErrorKind::AccessDenied
    )
}

/// Whether the call was cancelled by Fabro rather than failed by the provider.
#[must_use]
pub fn is_cancelled<E: ErrorFacts + ?Sized>(error: &E) -> bool {
    error.kind() == ErrorKind::Cancelled
}

/// Whether another provider is worth trying.
///
/// Everything retryable qualifies, plus failures that are local to this
/// provider: credentials, access policy, model inventory, quota, and a
/// provider that ran out of time. A different provider has its own.
#[must_use]
pub fn failover_eligible<E: ErrorFacts + ?Sized>(error: &E) -> bool {
    if is_retryable(error) {
        return true;
    }
    matches!(
        error.kind(),
        ErrorKind::Authentication
            | ErrorKind::AccessDenied
            | ErrorKind::NotFound
            | ErrorKind::QuotaExceeded
            | ErrorKind::RateLimit
            | ErrorKind::Server
            | ErrorKind::Network
            | ErrorKind::Timeout
            | ErrorKind::StreamDecode
    ) || (error.kind() == ErrorKind::ContentFilter && error.provider_code() == Some("refusal"))
}

/// A stable `category|provider|detail` string for loop and restart detection.
#[must_use]
pub fn failure_signature_hint<E: ErrorFacts + ?Sized>(error: &E) -> String {
    let provider = error.provider().map_or("unknown", ProviderId::as_str);
    let category = match error.kind() {
        ErrorKind::Cancelled => "api_canceled",
        _ if is_retryable(error) => "api_transient",
        _ => "api_deterministic",
    };
    let detail = error.kind().as_str().to_string();
    format!("{category}|{provider}|{detail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(kind: ErrorKind) -> Error {
        Error::new(kind, "boom").with_provider(ProviderId::new("openai"))
    }

    #[test]
    fn signatures_name_category_provider_and_kind() {
        assert_eq!(
            failure_signature_hint(&error(ErrorKind::InvalidRequest)),
            "api_deterministic|openai|invalid_request"
        );
        assert_eq!(
            failure_signature_hint(
                &error(ErrorKind::RateLimit).with_retry(RetryClassification::Safe)
            ),
            "api_transient|openai|rate_limit"
        );
        assert_eq!(
            failure_signature_hint(&error(ErrorKind::Cancelled)),
            "api_canceled|openai|cancelled"
        );
    }

    #[test]
    fn failover_covers_provider_local_failures() {
        assert!(failover_eligible(&error(ErrorKind::Authentication)));
        assert!(failover_eligible(&error(ErrorKind::QuotaExceeded)));
        assert!(!failover_eligible(&error(ErrorKind::InvalidRequest)));
        assert!(!failover_eligible(&error(ErrorKind::ContextLength)));
        assert!(!failover_eligible(&error(ErrorKind::ContentFilter)));
        assert!(failover_eligible(
            &error(ErrorKind::ContentFilter).with_provider_code("refusal")
        ));
    }

    #[test]
    fn stored_errors_keep_the_facts_and_round_trip() {
        let live = error(ErrorKind::RateLimit)
            .with_status(429)
            .with_provider_code("slow")
            .with_retry(RetryClassification::after(Duration::from_secs(2)))
            .with_source(std::io::Error::other("socket closed"));
        let stored = LlmError::from(&live);
        assert_eq!(stored.kind(), ErrorKind::RateLimit);
        assert_eq!(stored.status(), Some(429));
        assert_eq!(stored.provider_code(), Some("slow"));
        assert_eq!(stored.retry_after(), Some(Duration::from_secs(2)));
        assert_eq!(stored.source_message(), Some("socket closed"));
        assert_eq!(stored.to_string(), "boom");
        assert!(stored.is_retryable());
        assert_eq!(
            stored.failure_signature_hint(),
            failure_signature_hint(&live)
        );

        let json = serde_json::to_value(&stored).unwrap();
        assert_eq!(json["kind"], "rate_limit");
        let decoded: LlmError = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, stored);
    }
}
