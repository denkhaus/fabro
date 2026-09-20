//! Per-call first-token (TTFT) timeout for streaming LLM calls.
//!
//! A provider can hold a stream open without ever producing output — the
//! connection is alive, so no transport error fires, and the call hangs
//! until the provider deigns to answer (observed: a 163 s first-token wait,
//! run 01M2NVXCSQ5FCB51MV3Z8AA6CF). This middleware bounds that wait: a
//! stream that produces no visible event within the budget errors with a
//! retryable, failover-eligible `Timeout`, so the existing retry middleware
//! (which reconnects before visible output) and the fallback routing treat
//! it exactly like a provider error.
//!
//! Only streaming operations carry a first-token seam: a non-streaming
//! `complete` returns its whole response as one unit, so "first token" is
//! unobservable there and a wall-clock wrapper around the call would abort
//! healthy slow generations — the failure mode this timeout exists to
//! avoid. Complete calls pass through untouched.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use fabro_types::settings::DEFAULT_FIRST_TOKEN_TIMEOUT_SECS;
use futures::stream::{StreamExt as _, unfold};
use lithos_llm::catalog::ProviderId;
use lithos_llm::middleware::{Call, Middleware, Next, Operation, Output};
use lithos_llm::types::{Error, ErrorKind, ResponseStream, RetryClassification};
use tokio::time::{Instant, sleep_until};

/// The engine default: long enough for a loaded provider to think, short
/// enough that a stalled stream fails while a stage can still retry. The
/// value lives in `fabro-types`; this is its `Duration` form.
pub const DEFAULT_FIRST_TOKEN_TIMEOUT: Duration =
    Duration::from_secs(DEFAULT_FIRST_TOKEN_TIMEOUT_SECS);

/// Bounds how long a streaming call may run before its first visible
/// output event.
///
/// The clock starts when the call enters this middleware (per provider
/// attempt: the retry middleware re-runs the inner stack, so every attempt
/// gets a fresh budget). Once any visible event arrives, the wrapper steps
/// aside; mid-stream stalls are lithos's stream-idle timeout, not TTFT.
#[derive(Debug)]
pub struct FirstTokenTimeout {
    timeout: Duration,
}

impl FirstTokenTimeout {
    #[must_use]
    pub fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

#[async_trait]
impl Middleware for FirstTokenTimeout {
    async fn handle(&self, call: Call, next: Next) -> Result<Output, Error> {
        if call.operation() != Operation::Stream {
            return next.run(call).await;
        }
        let started = Instant::now();
        let provider = call.route().provider().id().clone();
        match next.run(call).await {
            Ok(Output::Stream(stream)) => Ok(Output::Stream(wrap(
                stream,
                self.timeout,
                started,
                provider,
            ))),
            output => output,
        }
    }
}

/// The error a stalled stream times out with. `Timeout` is
/// failover-eligible on its own; `Safe` also lets the retry middleware
/// reconnect on the same provider immediately.
fn timeout_error(timeout: Duration, provider: &ProviderId) -> Error {
    Error::new(
        ErrorKind::Timeout,
        format!("provider {provider} produced no first token within {timeout:?}"),
    )
    .with_provider(provider.clone())
    .with_retry(RetryClassification::Safe)
}

/// The state of one wrapped stream: the inner stream plus the deadline,
/// which is cleared by the first visible event.
struct TtftState {
    stream:   ResponseStream,
    deadline: Option<Instant>,
    timeout:  Duration,
    provider: ProviderId,
}

fn wrap(
    stream: ResponseStream,
    timeout: Duration,
    started: Instant,
    provider: ProviderId,
) -> ResponseStream {
    ResponseStream::new(unfold(
        TtftState {
            stream,
            deadline: Some(started + timeout),
            timeout,
            provider,
        },
        |mut state| async move {
            let Some(deadline) = state.deadline else {
                return state.stream.next().await.map(|item| (item, state));
            };
            // A ready event wins over an expired deadline, so a token that
            // raced the timer is delivered, not dropped.
            tokio::select! {
                biased;
                item = state.stream.next() => {
                    if matches!(&item, Some(Ok(event)) if event.is_visible()) {
                        state.deadline = None;
                    }
                    item.map(|item| (item, state))
                }
                () = sleep_until(deadline) => {
                    Some((Err(timeout_error(state.timeout, &state.provider)), state))
                }
            }
        },
    ))
}

/// Convenience: the middleware as an `Arc`, for
/// [`ClientOptions::with_middleware`](crate::ClientOptions::with_middleware).
#[must_use]
pub fn first_token_timeout(timeout: Duration) -> Arc<dyn Middleware> {
    Arc::new(FirstTokenTimeout::new(timeout))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use futures::{StreamExt as _, stream};
    use lithos_llm::adapter::{ProviderAdapter, ResolvedCall};
    use lithos_llm::catalog::AdapterId;
    use lithos_llm::client::Client;
    use lithos_llm::types::{
        ContentBlockId, Error, Message, Request, Response, ResponseStream, Role, StreamEvent,
    };
    use tokio::time::sleep;

    use super::{DEFAULT_FIRST_TOKEN_TIMEOUT, FirstTokenTimeout};
    use crate::client::ClientOptions;
    use crate::test_support::{
        client_with_adapters, response_to_stream, test_retry_policy, text_response,
    };
    use crate::types::ErrorKind;

    /// An adapter whose streams stall until `visible_after` attempts have
    /// passed, then answer normally; `complete` always answers after a
    /// beat slower than the timeouts under test.
    struct StallThenAnswer {
        id:            AdapterId,
        stream_calls:  AtomicUsize,
        visible_after: usize,
    }

    impl StallThenAnswer {
        fn always_stalled() -> Self {
            Self {
                id:            AdapterId::new("stall-then-answer"),
                stream_calls:  AtomicUsize::new(0),
                visible_after: usize::MAX,
            }
        }

        fn calls(&self) -> usize {
            self.stream_calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ProviderAdapter for StallThenAnswer {
        fn id(&self) -> &AdapterId {
            &self.id
        }

        async fn complete(&self, _call: &ResolvedCall) -> Result<Response, Error> {
            // Slower than every timeout under test: TTFT must not apply to
            // complete calls.
            sleep(Duration::from_millis(500)).await;
            Ok(text_response("openrouter", "kimi-k3", "complete"))
        }

        async fn stream(&self, _call: &ResolvedCall) -> Result<ResponseStream, Error> {
            let attempt = self.stream_calls.fetch_add(1, Ordering::SeqCst);
            if attempt < self.visible_after {
                return Ok(ResponseStream::new(stream::pending()));
            }
            Ok(response_to_stream(text_response(
                "openrouter",
                "kimi-k3",
                "hello",
            )))
        }
    }

    /// An adapter whose stream yields one visible delta immediately, then
    /// ends only after a delay far past every TTFT budget under test.
    struct ImmediateThenLate {
        id: AdapterId,
    }

    #[async_trait]
    impl ProviderAdapter for ImmediateThenLate {
        fn id(&self) -> &AdapterId {
            &self.id
        }

        async fn complete(&self, _call: &ResolvedCall) -> Result<Response, Error> {
            Ok(text_response("openrouter", "kimi-k3", "complete"))
        }

        async fn stream(&self, _call: &ResolvedCall) -> Result<ResponseStream, Error> {
            let delta = Ok(StreamEvent::TextDelta {
                id:   ContentBlockId::new("b0"),
                text: "first".to_string(),
            });
            let late_end = stream::once(async {
                sleep(Duration::from_secs(2)).await;
                let response = text_response("openrouter", "kimi-k3", "first");
                Ok::<StreamEvent, Error>(StreamEvent::Ended {
                    response: Box::new(response),
                })
            });
            Ok(ResponseStream::new(
                stream::iter(vec![delta]).chain(late_end),
            ))
        }
    }

    fn request() -> Request {
        Request::builder()
            .model("openrouter/kimi-k3")
            .message(Message::text(Role::User, "hi"))
            .build()
            .expect("test request should build")
    }

    fn ttft_client(adapter: Arc<dyn ProviderAdapter>, timeout: Duration, retry: bool) -> Client {
        let mut options =
            ClientOptions::default().with_middleware(Arc::new(FirstTokenTimeout::new(timeout)));
        if retry {
            options.retry = Some(test_retry_policy());
        }
        client_with_adapters(vec![("openrouter", adapter)], options)
    }

    async fn collect_text(stream: &mut ResponseStream) -> String {
        let mut text = String::new();
        while let Some(item) = stream.next().await {
            match item.expect("stream stays healthy") {
                StreamEvent::TextDelta { text: delta, .. } => text.push_str(&delta),
                StreamEvent::Ended { .. } => break,
                _ => {}
            }
        }
        text
    }

    #[test]
    fn default_is_tens_of_seconds_not_minutes() {
        assert!(DEFAULT_FIRST_TOKEN_TIMEOUT >= Duration::from_secs(20));
        assert!(DEFAULT_FIRST_TOKEN_TIMEOUT < Duration::from_mins(1));
    }

    #[tokio::test(start_paused = true)]
    async fn stalled_first_token_stream_times_out() {
        let adapter = Arc::new(StallThenAnswer::always_stalled());
        let client = ttft_client(Arc::clone(&adapter) as _, Duration::from_millis(200), false);

        let mut stream = client.stream(request()).await.expect("stream opens");

        let error = stream
            .next()
            .await
            .expect("timeout yields an error item")
            .expect_err("a stalled stream must error");
        assert_eq!(error.kind(), ErrorKind::Timeout);
        assert!(
            error.to_string().contains("first token"),
            "message names the timeout: {error}"
        );
        assert_eq!(adapter.calls(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn ttft_timeout_fires_the_retry_middleware_and_recovers() {
        // Attempt 1 stalls and is cut off; attempt 2 answers. The retry
        // middleware reconnects before visible output, so one logical
        // stream delivers attempt 2's answer: a TTFT timeout rides the
        // existing retry path like any provider error.
        let adapter = Arc::new(StallThenAnswer {
            id:            AdapterId::new("stall-then-answer"),
            stream_calls:  AtomicUsize::new(0),
            visible_after: 1,
        });
        let client = ttft_client(Arc::clone(&adapter) as _, Duration::from_millis(200), true);

        let mut stream = client.stream(request()).await.expect("stream opens");
        let text = collect_text(&mut stream).await;

        assert_eq!(adapter.calls(), 2, "the stalled attempt must be retried");
        assert_eq!(text, "hello");
    }

    #[tokio::test(start_paused = true)]
    async fn ttft_exhausting_retries_surfaces_the_timeout_error() {
        let adapter = Arc::new(StallThenAnswer::always_stalled());
        let client = ttft_client(Arc::clone(&adapter) as _, Duration::from_millis(200), true);

        let mut stream = client.stream(request()).await.expect("stream opens");
        let error = stream
            .next()
            .await
            .expect("terminal error item")
            .expect_err("exhausted retries must surface the TTFT timeout");
        assert_eq!(error.kind(), ErrorKind::Timeout);
        // test_retry_policy allows three attempts.
        assert_eq!(adapter.calls(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn visible_output_before_the_deadline_disarms_the_timeout() {
        // First token arrives immediately; the stream then waits far past
        // the budget before ending. The wrapper must step aside after the
        // first visible event — mid-stream pacing is not TTFT's business.
        let adapter = Arc::new(ImmediateThenLate {
            id: AdapterId::new("immediate-then-late"),
        });
        let client = ttft_client(Arc::clone(&adapter) as _, Duration::from_millis(200), false);

        let mut stream = client.stream(request()).await.expect("stream opens");
        let text = collect_text(&mut stream).await;

        assert_eq!(text, "first");
    }

    #[tokio::test(start_paused = true)]
    async fn non_streaming_complete_is_not_bounded_by_ttft() {
        let adapter = Arc::new(StallThenAnswer::always_stalled());
        let client = ttft_client(Arc::clone(&adapter) as _, Duration::from_millis(200), false);

        // The adapter's complete waits 500 ms, past the 200 ms budget; a
        // complete call has no first-token seam, so it must still succeed.
        let response = client
            .complete(request())
            .await
            .expect("complete is unbounded");
        assert_eq!(response.text(), "complete");
    }

    #[test]
    fn constructor_exposes_the_budget() {
        let middleware = FirstTokenTimeout::new(DEFAULT_FIRST_TOKEN_TIMEOUT);
        assert_eq!(middleware.timeout(), DEFAULT_FIRST_TOKEN_TIMEOUT);
    }
}
