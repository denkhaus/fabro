//! The Fabro server completions gateway as a lithos provider adapter.
//!
//! `fabro exec --server` sends every model call to `POST /api/v1/completions`
//! on a Fabro server, which holds the provider credentials and the catalog
//! and is the billing authority. The server returns lithos `Response` JSON
//! and streams lithos `StreamEvent` JSON verbatim, so this adapter decodes
//! the standard types and trusts the cost inside them.
//!
//! Transport (authentication, token refresh, base URL) belongs to the caller
//! through [`GatewayTransport`], so this crate does not depend on the CLI's
//! server client.

use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use chrono::NaiveDateTime;
use fabro_http::HeaderMap;
use futures::{StreamExt as _, stream};
use lithos_llm::adapter::{ProviderAdapter, ResolvedCall};
use lithos_llm::catalog::{AdapterId, ProviderId};
use lithos_llm::types::{
    Error, ErrorKind, Response, ResponseStream, RetryClassification, StreamEvent,
};

/// Adapter id reported for gateway routes.
pub const GATEWAY_ADAPTER_ID: &str = "fabro-gateway";

/// How the adapter reaches the server.
#[async_trait]
pub trait GatewayTransport: Send + Sync {
    /// Posts a completion body and returns the raw HTTP response.
    async fn post_completion(
        &self,
        body: serde_json::Value,
    ) -> Result<fabro_http::Response, GatewayError>;
}

/// A failure between the adapter and the server.
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    /// The request never produced an HTTP response.
    #[error("{message}")]
    Transport {
        message: String,
        /// Whether the failure was a missing or rejected Fabro login.
        auth:    bool,
    },
    /// The server answered with an error status.
    #[error("server returned HTTP {status}")]
    Status {
        status:  u16,
        headers: HeaderMap,
        body:    String,
    },
}

pub struct GatewayAdapter {
    id:        AdapterId,
    transport: Box<dyn GatewayTransport>,
}

impl GatewayAdapter {
    #[must_use]
    pub fn new(transport: Box<dyn GatewayTransport>) -> Self {
        Self {
            id: AdapterId::new(GATEWAY_ADAPTER_ID),
            transport,
        }
    }

    fn body(call: &ResolvedCall, stream: bool) -> Result<serde_json::Value, Error> {
        let mut body = serde_json::to_value(call.request()).map_err(|source| {
            Error::new(ErrorKind::InvalidRequest, "failed to serialize request").with_source(source)
        })?;
        // The gateway resolves models itself; send the canonical route so the
        // server and the local catalog agree on the offering.
        body["model"] = serde_json::Value::String(call.route().handle().to_string());
        body["stream"] = serde_json::Value::Bool(stream);
        Ok(body)
    }

    async fn send(&self, call: &ResolvedCall, stream: bool) -> Result<fabro_http::Response, Error> {
        let provider = call.route().provider().id().clone();
        self.transport
            .post_completion(Self::body(call, stream)?)
            .await
            .map_err(|err| gateway_error(err, &provider))
    }
}

fn gateway_error(err: GatewayError, provider: &ProviderId) -> Error {
    match err {
        GatewayError::Transport { message, auth } => {
            let kind = if auth {
                ErrorKind::Authentication
            } else {
                ErrorKind::Network
            };
            let mut error = Error::new(kind, message).with_provider(provider.clone());
            if !auth {
                error = error.with_retry(RetryClassification::Safe);
            }
            error
        }
        GatewayError::Status {
            status,
            headers,
            body,
        } => {
            let (message, code) = parse_server_error_body(&body);
            let kind = match status {
                400 | 422 => ErrorKind::InvalidRequest,
                401 => ErrorKind::Authentication,
                403 => ErrorKind::AccessDenied,
                404 => ErrorKind::NotFound,
                408 | 504 => ErrorKind::Timeout,
                429 => ErrorKind::RateLimit,
                500..=599 => ErrorKind::Server,
                _ => ErrorKind::Provider,
            };
            let mut error = Error::new(kind.clone(), message.clone())
                .with_provider(provider.clone())
                .with_status(status);
            if let Some(code) = code {
                error = error.with_provider_code(code);
            }
            match kind {
                ErrorKind::RateLimit => {
                    error = error.with_retry(RetryClassification::Safe);
                    // The header wins when the provider sent one; otherwise
                    // the reset deadline may still live in the message prose.
                    let after =
                        retry_after(&headers).or_else(|| reset_window(&message, SystemTime::now()));
                    if let Some(after) = after {
                        error = error
                            .with_retry(RetryClassification::after(after))
                            .with_provider_retry_after(after);
                    }
                }
                ErrorKind::Server | ErrorKind::Timeout => {
                    error = error.with_retry(RetryClassification::Safe);
                    if let Some(after) = retry_after(&headers) {
                        error = error
                            .with_retry(RetryClassification::after(after))
                            .with_provider_retry_after(after);
                    }
                }
                _ => {}
            }
            error
        }
    }
}

fn retry_after(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<f64>().ok())
        .map(Duration::from_secs_f64)
}

/// The timestamp format providers embed in usage-window reset prose.
const RESET_TIMESTAMP_LEN: usize = "YYYY-MM-DD HH:MM:SS".len();

/// Parses a provider usage-window reset deadline out of error message text.
///
/// Some providers (zai and Anthropic-style 429 bodies) say when a usage
/// window reopens only in prose: "Usage limit reached for 5 hour. Your limit
/// will reset at 2026-09-03 05:35:16". The timestamp is timezone-naive and
/// is read as UTC (fabro-a3d8).
#[must_use]
pub fn parse_reset_deadline(message: &str) -> Option<NaiveDateTime> {
    let marker = "will reset at ";
    let start = message.find(marker)? + marker.len();
    let timestamp = message.get(start..)?.get(..RESET_TIMESTAMP_LEN)?;
    NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S").ok()
}

/// The wait until the reset deadline parsed from `message`, when one exists
/// and lies in the future at `now`.
///
/// A deadline already reached advises nothing — the window has reopened, so
/// ordinary short-window retry behavior applies.
#[must_use]
pub fn reset_window(message: &str, now: SystemTime) -> Option<Duration> {
    let deadline = parse_reset_deadline(message)?;
    let now = chrono::DateTime::<chrono::Utc>::from(now).naive_utc();
    deadline
        .signed_duration_since(now)
        .to_std()
        .ok()
        .filter(|window| !window.is_zero())
}

/// Reads the Fabro API error envelope (`errors[0].detail` / `code`), falling
/// back to the raw body.
#[must_use]
pub fn parse_server_error_body(body: &str) -> (String, Option<String>) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return (body.to_string(), None);
    };
    let first = value
        .get("errors")
        .and_then(serde_json::Value::as_array)
        .and_then(|errors| errors.first());
    let detail = first
        .and_then(|entry| entry.get("detail"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.get("detail").and_then(serde_json::Value::as_str))
        .unwrap_or("Unknown error")
        .to_string();
    let code = first
        .and_then(|entry| entry.get("code"))
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned);
    (detail, code)
}

fn parse_sse_block(block: &str) -> Option<(String, String)> {
    let mut event_type = None;
    let mut data_lines = Vec::new();
    for line in block.lines() {
        if let Some(value) = line.strip_prefix("event:") {
            event_type = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim());
        }
    }
    let event_type = event_type?;
    (!data_lines.is_empty()).then(|| (event_type, data_lines.join("\n")))
}

fn decode_error(message: String, source: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::new(ErrorKind::StreamDecode, message).with_source(source)
}

#[async_trait]
impl ProviderAdapter for GatewayAdapter {
    fn id(&self) -> &AdapterId {
        &self.id
    }

    async fn complete(&self, call: &ResolvedCall) -> Result<Response, Error> {
        let response = self.send(call, false).await?;
        let body = response.text().await.map_err(|source| {
            Error::new(ErrorKind::Network, "failed to read completion body")
                .with_source(source)
                .with_retry(RetryClassification::Safe)
        })?;
        serde_json::from_str(&body)
            .map_err(|source| decode_error("failed to parse completion response".into(), source))
    }

    async fn stream(&self, call: &ResolvedCall) -> Result<ResponseStream, Error> {
        let response = self.send(call, true).await?;
        let state = SseState {
            buffer: String::new(),
            bytes:  Box::pin(response.bytes_stream()),
        };
        let events = stream::unfold(state, |mut state| async move {
            loop {
                if let Some(position) = state.buffer.find("\n\n") {
                    let block = state.buffer[..position].to_string();
                    state.buffer = state.buffer[position + 2..].to_string();
                    let Some((event_type, data)) = parse_sse_block(&block) else {
                        continue;
                    };
                    if event_type != "stream_event" {
                        continue;
                    }
                    let event = serde_json::from_str::<StreamEvent>(&data).map_err(|source| {
                        decode_error("failed to parse stream event".into(), source)
                    });
                    return Some((event, state));
                }
                match state.bytes.next().await {
                    Some(Ok(chunk)) => state.buffer.push_str(&String::from_utf8_lossy(&chunk)),
                    Some(Err(source)) => {
                        let error = Error::new(ErrorKind::Network, "stream read failed")
                            .with_source(source)
                            .with_retry(RetryClassification::Safe);
                        return Some((Err(error), state));
                    }
                    None => return None,
                }
            }
        });
        Ok(ResponseStream::new(events))
    }
}

type ByteStream = std::pin::Pin<
    Box<dyn futures::Stream<Item = Result<bytes::Bytes, fabro_http::HttpError>> + Send>,
>;

struct SseState {
    buffer: String,
    bytes:  ByteStream,
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::client::default_retry_policy;

    /// The zai 429 body from the fabro-a3d8 incident: `retry_after=null`,
    /// the reset deadline only in message prose.
    const ZAI_USAGE_LIMIT: &str =
        "Usage limit reached for 5 hour. Your limit will reset at 2026-09-03 05:35:16";

    fn utc(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, second)
            .single()
            .expect("valid test timestamp")
    }

    #[test]
    fn reset_deadline_is_parsed_from_zai_usage_prose() {
        assert_eq!(
            parse_reset_deadline(ZAI_USAGE_LIMIT).map(|deadline| deadline.and_utc()),
            Some(utc(2026, 9, 3, 5, 35, 16))
        );
        // The Anthropic-style wording without the window sentence parses too.
        assert_eq!(
            parse_reset_deadline("Your limit will reset at 2026-09-03 05:35:16")
                .map(|deadline| deadline.and_utc()),
            Some(utc(2026, 9, 3, 5, 35, 16))
        );
    }

    #[test]
    fn reset_prose_without_a_timestamp_yields_none() {
        assert!(parse_reset_deadline("rate limit exceeded, slow down").is_none());
        assert!(parse_reset_deadline("Your limit will reset at soon").is_none());
        assert!(parse_reset_deadline("").is_none());
    }

    #[test]
    fn reset_window_measures_from_now_and_drops_past_deadlines() {
        let now = utc(2026, 9, 3, 1, 35, 16);
        assert_eq!(
            reset_window(ZAI_USAGE_LIMIT, SystemTime::from(now)),
            Some(Duration::from_hours(4))
        );
        // At or past the deadline there is nothing left to wait for.
        let later = utc(2026, 9, 3, 5, 35, 16);
        assert_eq!(reset_window(ZAI_USAGE_LIMIT, SystemTime::from(later)), None);
        let much_later = utc(2026, 9, 4, 0, 0, 0);
        assert_eq!(
            reset_window(ZAI_USAGE_LIMIT, SystemTime::from(much_later)),
            None
        );
    }

    /// A reset deadline safely in the future for any test run date.
    fn far_future_reset() -> String {
        let future =
            chrono::DateTime::<chrono::Utc>::from(SystemTime::now()) + chrono::Duration::hours(3);
        future.format("%Y-%m-%d %H:%M:%S").to_string()
    }

    #[test]
    fn server_error_envelope_is_parsed() {
        let (detail, code) = parse_server_error_body(
            r#"{"errors":[{"status":"429","title":"Too Many","detail":"slow down","code":"rate"}]}"#,
        );
        assert_eq!(detail, "slow down");
        assert_eq!(code.as_deref(), Some("rate"));
        let (detail, code) = parse_server_error_body("plain text");
        assert_eq!(detail, "plain text");
        assert!(code.is_none());
    }

    #[test]
    fn sse_blocks_split_event_and_data() {
        assert_eq!(
            parse_sse_block("event: stream_event\ndata: {\"a\":1}"),
            Some(("stream_event".to_string(), "{\"a\":1}".to_string()))
        );
        assert_eq!(parse_sse_block(": comment"), None);
    }

    #[test]
    fn status_codes_map_to_error_kinds() {
        let provider = ProviderId::new("openai");
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", "2".parse().unwrap());
        let error = gateway_error(
            GatewayError::Status {
                status: 429,
                headers,
                body: String::new(),
            },
            &provider,
        );
        assert_eq!(error.kind(), ErrorKind::RateLimit);
        assert_eq!(error.retry_after(), Some(Duration::from_secs(2)));
        let error = gateway_error(
            GatewayError::Transport {
                message: "login required".into(),
                auth:    true,
            },
            &provider,
        );
        assert_eq!(error.kind(), ErrorKind::Authentication);
        assert_eq!(error.retry_classification(), RetryClassification::Never);
    }

    #[test]
    fn rate_limit_prose_attaches_the_reset_window() {
        let provider = ProviderId::new("zai");
        // The incident wording with a deadline still in the future so the
        // measured window is hours, not empty.
        let message = format!(
            "Usage limit reached for 5 hour. Your limit will reset at {}",
            far_future_reset()
        );
        let body =
            serde_json::json!({ "errors": [{ "status": "429", "detail": message }] }).to_string();
        let error = gateway_error(
            GatewayError::Status {
                status: 429,
                headers: HeaderMap::new(),
                body,
            },
            &provider,
        );
        let window = error.provider_retry_after().expect("parsed reset window");
        assert!(
            window >= Duration::from_hours(1),
            "a five-hour usage window must parse as hours, got {window:?}"
        );
        assert_eq!(error.retry_after(), Some(window));
        // A multi-hour window is beyond any count-based backoff budget: the
        // default policy must refuse the next attempt and surface the error
        // immediately rather than burning the remaining attempts.
        assert_eq!(default_retry_policy().next_delay(1, &error), None);
    }

    #[test]
    fn retry_after_header_wins_over_message_prose() {
        let provider = ProviderId::new("zai");
        let mut headers = HeaderMap::new();
        headers.insert("retry-after", "3".parse().unwrap());
        let error = gateway_error(
            GatewayError::Status {
                status: 429,
                headers,
                body: serde_json::json!({ "detail": ZAI_USAGE_LIMIT }).to_string(),
            },
            &provider,
        );
        assert_eq!(error.retry_after(), Some(Duration::from_secs(3)));
    }
}
