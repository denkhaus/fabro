//! The judgment provider (ADR-0022 wave 3, fabro-b506): System One's
//! Decisions API as a first-class fabro-llm client.
//!
//! A judgment call is not a chat completion: it posts a `state` plus typed
//! questions (`choice` / `score` / `noul`) and receives typed answers with
//! probabilities and confidence, input-only billed. That wire contract has
//! its own endpoint (OpenRouter's System One surface today; the direct
//! TypeSafe API once its waitlist opens), so this module owns a small typed
//! client rather than bending the lithos completion pipeline. What it shares
//! with every other provider call is the retry policy: the loop drives
//! [`RetryPolicy::next_delay`] — the same whole-retry-decision the lithos
//! middleware uses — and reports each retry through the crate's
//! [`RetryListener`] path ([`crate::client`]).
//!
//! Secrets: the API key arrives as a parameter from server env per the
//! server-secrets strategy; this module never reads env itself, and the key
//! never enters a run sandbox or a prompt.
//!
//! The model catalog entry lives in `fork-catalog-overlay.toml`
//! (openrouter `typesafe/jev-latest`); the `kind = "judgment"` marker is
//! fabro-side metadata — lithos's catalog schema has no kind field — read by
//! [`is_judgment_model`].

use std::collections::BTreeMap;

use lithos_llm::catalog::CatalogModel;
use lithos_llm::middleware::{RetryPolicy, RetryStage};
use lithos_llm::types::{Error as LlmError, ErrorData, ErrorKind, RetryClassification};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::{RetryListener, RetryNotice};

/// The System One endpoint Fabro points at today (operator repoint
/// 2026-09-24): OpenRouter maps bare System One model ids onto the
/// `typesafe/` namespace. Configurable through [`JudgmentEndpoint`] for the
/// direct TypeSafe API later.
pub const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1/systemone";

/// The wire model id, version-pinned so threshold tuning cannot drift with
/// an alias (ADR-0022). The catalog entry's id is
/// `openrouter/typesafe/jev-latest`; the wire spelling OpenRouter expects is
/// the bare id.
pub const DEFAULT_MODEL: &str = "jev-latest";

/// The metadata namespace Fabro's own catalog markers live under.
const FABRO_METADATA_NAMESPACE: &str = "fabro";

/// The catalog marker value that names a judgment model.
const JUDGMENT_KIND: &str = "judgment";

/// Where the judgment call goes and which model answers.
#[derive(Clone, Debug)]
pub struct JudgmentEndpoint {
    /// The Decisions API endpoint (see [`DEFAULT_BASE_URL`]).
    pub base_url: String,
    /// The wire model id (see [`DEFAULT_MODEL`]).
    pub model:     String,
}

impl Default for JudgmentEndpoint {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_string(),
            model:    DEFAULT_MODEL.to_string(),
        }
    }
}

/// One typed question, tagged by `type` on the wire.
///
/// `choice` carries its options as a criteria map (option id -> meaning);
/// `score` carries its rubric the same way; `noul` (no output labels)
/// answers without a fixed option set.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Question {
    Choice {
        instructions: String,
        criteria:     BTreeMap<String, String>,
    },
    Score {
        instructions: String,
        criteria:     BTreeMap<String, String>,
    },
    Noul {
        instructions: String,
    },
}

/// One request body: the model, the evidence state, and the questions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JudgmentRequest {
    pub model:     String,
    pub state:     Value,
    pub questions: BTreeMap<String, Question>,
}

impl JudgmentRequest {
    /// Builds the request `client.judge` would send for `state` and
    /// `questions` under `model`.
    #[must_use]
    pub fn new(model: impl Into<String>, state: Value, questions: BTreeMap<String, Question>) -> Self {
        Self {
            model:     model.into(),
            state,
            questions,
        }
    }
}

/// The answer to one question: the selected option (or score), the
/// probability mass over the criteria, and the model's confidence.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    pub answer:        AnswerValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probabilities: Option<BTreeMap<String, f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence:    Option<f64>,
}

/// The selected value: an option id for `choice`/`noul` questions, a number
/// for `score` questions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AnswerValue {
    Choice(String),
    Score(f64),
}

/// The usage block. OpenRouter's System One response reports
/// `usage.cost` (the direct TypeSafe surface called it `cost_usd`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct JudgmentUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
}

/// The response body: answers keyed by question id, plus usage.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct JudgmentResponse {
    pub answers: BTreeMap<String, Answer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage:   Option<JudgmentUsage>,
}

/// A judgment call failed. The inner error is a lithos
/// [`Error`](lithos_llm::types::Error) so retry classifications, status
/// codes, and messages match every other provider failure path.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct JudgmentError(#[from] LlmError);

impl JudgmentError {
    /// The provider-neutral failure, for callers that report or journal it.
    #[must_use]
    pub fn as_llm_error(&self) -> &LlmError {
        &self.0
    }
}

/// The System One judgment client.
///
/// Holds an HTTP client, the endpoint/model pair, the retry policy (the
/// same [`RetryPolicy`] the lithos middleware runs, driven through
/// `next_delay` here because a judgment call owns its whole loop), and an
/// optional [`RetryListener`] that observes each retry.
pub struct JudgmentClient {
    http:     fabro_http::HttpClient,
    endpoint: JudgmentEndpoint,
    policy:   RetryPolicy,
    listener: Option<RetryListener>,
}

impl JudgmentClient {
    /// Builds a client for `endpoint` with `policy` and an optional retry
    /// listener.
    #[must_use]
    pub fn new(
        http: fabro_http::HttpClient,
        endpoint: JudgmentEndpoint,
        policy: RetryPolicy,
        listener: Option<RetryListener>,
    ) -> Self {
        Self {
            http,
            endpoint,
            policy,
            listener,
        }
    }

    /// Asks the judgment model: sends `state` plus `questions` to the
    /// configured endpoint and returns the typed answers.
    ///
    /// `api_key` comes from server env only (server-secrets strategy). The
    /// call retries on the policy's schedule — rate limits, server errors,
    /// and network faults — and reports each retry to the listener.
    ///
    /// # Errors
    /// [`JudgmentError`] when the endpoint rejects the request (after
    /// retries, for retryable failures) or the response body does not
    /// decode.
    pub async fn judge(
        &self,
        api_key: &str,
        state: Value,
        questions: BTreeMap<String, Question>,
    ) -> Result<JudgmentResponse, JudgmentError> {
        let request = JudgmentRequest::new(self.endpoint.model.clone(), state, questions);
        let mut attempt = 1_u32;
        loop {
            let error = match self.attempt(api_key, &request).await {
                Ok(response) => return Ok(response),
                Err(error) => error,
            };
            let Some(delay) = self.policy.next_delay(attempt, &error) else {
                return Err(error.into());
            };
            if let Some(listener) = &self.listener {
                listener.notify(RetryNotice {
                    error: ErrorData::from(&error),
                    attempt,
                    delay,
                    stage: RetryStage::Request,
                });
            }
            attempt += 1;
            tokio::time::sleep(delay).await;
        }
    }

    /// One HTTP attempt, mapping every failure onto a lithos error with a
    /// retry classification the policy can read.
    async fn attempt(
        &self,
        api_key: &str,
        request: &JudgmentRequest,
    ) -> Result<JudgmentResponse, LlmError> {
        let response = self
            .http
            .post(&self.endpoint.base_url)
            .bearer_auth(api_key)
            .json(request)
            .send()
            .await
            .map_err(|error| {
                LlmError::new(ErrorKind::Network, error.to_string())
                    .with_retry(RetryClassification::Safe)
            })?;
        let status = response.status();
        if !status.is_success() {
            let code = status.as_u16();
            let (kind, retry) = match code {
                401 | 403 => (ErrorKind::Authentication, RetryClassification::Never),
                404 => (ErrorKind::NotFound, RetryClassification::Never),
                429 => (ErrorKind::RateLimit, RetryClassification::Safe),
                500.. => (ErrorKind::Server, RetryClassification::Safe),
                _ => (ErrorKind::InvalidRequest, RetryClassification::Never),
            };
            let message = response
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(300)
                .collect::<String>();
            return Err(LlmError::new(kind, message)
                .with_status(code)
                .with_retry(retry));
        }
        response.json().await.map_err(|error| {
            LlmError::new(ErrorKind::ResponseDecode, error.to_string())
                .with_retry(RetryClassification::Safe)
        })
    }
}

/// Whether a catalog model is a judgment model — the fabro-side
/// `kind = "judgment"` marker under the `fabro` metadata namespace, which
/// lithos's kindless catalog schema cannot carry itself.
#[must_use]
pub fn is_judgment_model(model: &CatalogModel) -> bool {
    model
        .metadata()
        .get(FABRO_METADATA_NAMESPACE)
        .is_some_and(|namespace| {
            namespace.get("kind").and_then(Value::as_str) == Some(JUDGMENT_KIND)
        })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::Mutex;

    use fabro_test::test_http_client;
    use httpmock::MockServer;
    use lithos_llm::middleware::RetryPolicy;
    use lithos_llm::types::ErrorKind;
    use serde_json::json;
    use tokio::time::Duration;

    use crate::client::{RetryListener, RetryNotice};
    use crate::judgment::{
        AnswerValue, JudgmentClient, JudgmentEndpoint, JudgmentResponse, Question,
    };

    use super::{JudgmentRequest, is_judgment_model};

    /// The wire model id the hook and the provider agree on (operator
    /// repoint 2026-09-24): bare `jev-latest`, no alias drift.
    const WIRE_MODEL: &str = "jev-latest";

    fn verdict_question() -> (String, Question) {
        (
            "verdict_pre_screen".to_string(),
            Question::Choice {
                instructions: "Adjudicate the reviewed change.".to_string(),
                criteria:     BTreeMap::from([
                    ("approved".to_string(), "The change satisfies its spec.".to_string()),
                    (
                        "changes_requested".to_string(),
                        "The change has gaps.".to_string(),
                    ),
                ]),
            },
        )
    }

    #[test]
    fn request_round_trips_choice_score_noul() {
        let request = JudgmentRequest::new(
            WIRE_MODEL,
            json!({"run_id": "r1", "node": "reviewer"}),
            BTreeMap::from([
                verdict_question(),
                (
                    "flakiness".to_string(),
                    Question::Score {
                        instructions: "Score the failure's flakiness.".to_string(),
                        criteria:     BTreeMap::from([(
                            "deterministic".to_string(),
                            "Same tree, same result.".to_string(),
                        )]),
                    },
                ),
                (
                    "free_form".to_string(),
                    Question::Noul {
                        instructions: "What changed?".to_string(),
                    },
                ),
            ]),
        );
        let wire = serde_json::to_value(&request).expect("request serializes");
        assert_eq!(wire["model"], WIRE_MODEL);
        assert_eq!(wire["questions"]["verdict_pre_screen"]["type"], "choice");
        assert_eq!(wire["questions"]["flakiness"]["type"], "score");
        assert_eq!(wire["questions"]["free_form"]["type"], "noul");
        let back: JudgmentRequest =
            serde_json::from_value(wire).expect("request round-trips");
        assert_eq!(back, request);
    }

    #[test]
    fn response_round_trips_probabilities_confidence_usage() {
        let body = r#"{
            "answers": {
                "verdict_pre_screen": {
                    "answer": "approved",
                    "probabilities": {"approved": 0.91, "changes_requested": 0.09},
                    "confidence": 0.88
                },
                "flakiness": {
                    "answer": 0.4,
                    "confidence": 0.7
                }
            },
            "usage": {"cost": 0.00003}
        }"#;
        let response: JudgmentResponse =
            serde_json::from_str(body).expect("response decodes");
        let verdict = &response.answers["verdict_pre_screen"];
        assert_eq!(verdict.answer, AnswerValue::Choice("approved".to_string()));
        assert_eq!(
            verdict.probabilities.as_ref().and_then(|p| p.get("approved")),
            Some(&0.91)
        );
        assert_eq!(verdict.confidence, Some(0.88));
        assert_eq!(
            response.answers["flakiness"].answer,
            AnswerValue::Score(0.4)
        );
        assert_eq!(response.usage.as_ref().and_then(|u| u.cost), Some(0.00003));
        let wire = serde_json::to_value(&response).expect("response serializes");
        let back: JudgmentResponse =
            serde_json::from_value(wire).expect("response round-trips");
        assert_eq!(back, response);
    }

    /// Presence pin (fork feature, merge-upstream seam): the fork overlay
    /// registers the judgment model and its fabro-side kind marker, prices
    /// it input-only, and keeps the wire id bare.
    #[test]
    fn judgment_model_is_registered_input_only_and_marked() {
        let catalog = crate::build_catalog(&fabro_config::LlmLayer::default(), &|_| None)
            .expect("catalog builds");
        let model = catalog
            .model("openrouter", "typesafe/jev-latest")
            .expect("openrouter typesafe/jev-latest is registered");
        assert!(is_judgment_model(model), "kind=judgment marker present");
        assert_eq!(model.api_model(), WIRE_MODEL, "wire id stays bare");
        let pricing = model.pricing().expect("pricing present");
        assert_eq!(pricing.input_usd_micros_per_million, Some(42_000));
        assert_eq!(
            pricing.output_usd_micros_per_million, None,
            "judgment billing is input-only"
        );
        let ordinary = catalog
            .model("openrouter", "glm-5.3")
            .expect("ordinary openrouter model exists");
        assert!(
            !is_judgment_model(ordinary),
            "ordinary models are not judgment models"
        );
    }

    /// Twin (scripted double, no live OpenRouter, no credentials): the
    /// happy path sends auth + typed body and decodes the answers.
    #[tokio::test]
    async fn judge_sends_state_and_questions_and_decodes_answers() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST)
                    .path("/systemone")
                    .header("Authorization", "Bearer test-key")
                    .json_body_includes("\"model\":\"jev-latest\"")
                    .json_body_includes("\"run_id\":\"r1\"")
                    .json_body_includes("\"verdict_pre_screen\":{\"type\":\"choice\"");
                then.status(200).json_body(json!({
                    "answers": {
                        "verdict_pre_screen": {
                            "answer": "approved",
                            "probabilities": {"approved": 0.9},
                            "confidence": 0.8
                        }
                    },
                    "usage": {"cost": 0.00003}
                }));
            })
            .await;
        let client = JudgmentClient::new(
            test_http_client(),
            JudgmentEndpoint {
                base_url: server.url("/systemone"),
                model:    WIRE_MODEL.to_string(),
            },
            fast_policy(),
            None,
        );
        let response = client
            .judge(
                "test-key",
                json!({"run_id": "r1"}),
                BTreeMap::from([verdict_question()]),
            )
            .await
            .expect("judgment succeeds");
        assert_eq!(
            response.answers["verdict_pre_screen"].answer,
            AnswerValue::Choice("approved".to_string())
        );
        mock.assert_async().await;
    }

    /// Twin: a 500 then 200 retries through the policy and reports each
    /// retry on the RetryListener path.
    #[tokio::test]
    async fn judge_retries_server_errors_and_notifies_listener() {
        let server = MockServer::start_async().await;
        let fail = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::POST).path("/");
                then.status(500).body("upstream exploded");
            })
            .await;
        let ok = server
            .mock_async(|_, then| {
                then.status(200).json_body(json!({
                    "answers": {"flakiness": {"answer": 0.9, "confidence": 0.99}}
                }));
            })
            .await;
        let notices = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&notices);
        let client = JudgmentClient::new(
            test_http_client(),
            JudgmentEndpoint {
                base_url: server.url("/"),
                model:    WIRE_MODEL.to_string(),
            },
            fast_policy(),
            Some(RetryListener::new(move |notice: RetryNotice| {
                sink.lock().expect("notice sink").push(notice);
            })),
        );
        let response = client
            .judge("test-key", json!({}), BTreeMap::from([(
                "flakiness".to_string(),
                Question::Score {
                    instructions: "Score flakiness.".to_string(),
                    criteria:     BTreeMap::new(),
                },
            )]))
            .await
            .expect("judgment succeeds after retry");
        assert_eq!(
            response.answers["flakiness"].answer,
            AnswerValue::Score(0.9)
        );
        let notices = notices.lock().expect("notices");
        assert_eq!(notices.len(), 1, "the 500 produced one retry notice");
        assert_eq!(notices[0].attempt, 1);
        assert_eq!(
            notices[0].error.kind,
            ErrorKind::Server,
            "the 500 maps to a server error"
        );
        fail.assert_calls_async(1).await;
        ok.assert_calls_async(1).await;
    }

    /// Twin: an auth failure never retries.
    #[tokio::test]
    async fn judge_does_not_retry_authentication_failures() {
        let server = MockServer::start_async().await;
        let mock = server
            .mock_async(|_, then| {
                then.status(401).body("bad key");
            })
            .await;
        let client = JudgmentClient::new(
            test_http_client(),
            JudgmentEndpoint {
                base_url: server.url("/"),
                model:    WIRE_MODEL.to_string(),
            },
            fast_policy(),
            None,
        );
        let error = client
            .judge("wrong-key", json!({}), BTreeMap::from([verdict_question()]))
            .await
            .expect_err("auth fails");
        assert_eq!(error.as_llm_error().kind(), ErrorKind::Authentication);
        mock.assert_calls_async(1).await;
    }

    /// A fast policy so twin retry tests do not sleep the real backoff.
    fn fast_policy() -> RetryPolicy {
        RetryPolicy::exponential()
            .max_attempts(3)
            .initial_delay(Duration::from_millis(1))
            .max_delay(Duration::from_millis(2))
    }
}
