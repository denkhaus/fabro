//! Petri's `Interviewer` over Fabro's questions API and the worker's
//! control channel.
//!
//! A human gate in a Petri run asks through Petri's interview boundary: the
//! dispatcher hands this adapter one [`InterviewRequest`] per question, on
//! its own task, with the question's identity (invocation path, execution,
//! firing, attempt, node, occurrence, ask). The adapter surfaces the
//! question to Fabro the way a legacy `human` stage does, waits for the
//! answer the way the legacy worker does, and hands Petri the reply.
//!
//! # How a question reaches a person
//!
//! The legacy stage emits `interview.started` on the run's event stream;
//! the read side keeps it in the projection's `pending_interviews`, keyed
//! by question id, and that is what `GET /runs/{id}/questions`, the web
//! app's interview dock and the Slack integration read pending questions
//! from. This adapter posts the same event through a [`QuestionSink`]: the
//! worker's [`EventSinkQuestions`] appends it over the run event sink the
//! worker already carries lifecycle events on, and the server's in-process
//! path appends it through [`DatabaseQuestions`]. The question id is
//! Fabro's key for the question and is derived from Petri's identity
//! ([`question_id`]); the node name is the event's `stage`, and the Fabro
//! question type, options, freeform flag, deadline and review target are
//! mapped from Petri's [`Question`].
//!
//! # How the answer comes back
//!
//! `POST /runs/{id}/questions/{qid}/answer` validates the answer against
//! the pending record and delivers it to the run: over the worker control
//! bus as an `interview.answer` message, which the worker's control
//! manager applies to its [`ControlInterviewer`] by question id, or
//! straight to that interviewer for a run in the server process. The
//! adapter waits on that interviewer under the same id, so an answer
//! submitted before the wait began is buffered and one submitted after it
//! is delivered. The legacy answer shape is mapped onto Petri's
//! [`Answer`]: `yes` and `no` name the gate's affirmative and negative
//! choices by key, a selection names its key, a multi-selection its keys,
//! free text is text. A cancelled or interrupted answer ends the interview
//! without one: Petri's gate fails closed on it.
//!
//! # Expiry and cancellation
//!
//! The gate owns its answer deadline (Fabro's default when the node names
//! none) and reports the expiry itself; the dispatcher then fires the
//! adapter's cancel token, as it does when the firing ends without an
//! answer or the run is cancelled. The adapter returns promptly with
//! [`InterviewReply::Cancelled`] and posts `interview.timeout` when the
//! gate reported the expiry, else `interview.interrupted`, so the pending
//! question clears from Fabro's view. The expiry report is seen by the
//! adapter's own observer ([`FabroInterviewer::observer`]), which the run
//! registers ahead of the dispatcher so the report is noted before the
//! token fires. The dispatcher races the reply against the same token and
//! may drop the reply future the moment the token fires, so the notice is
//! posted from a guard that runs whether the future completes or is
//! dropped, on a task of its own. The dispatcher's own record of the
//! outcome (`TimedOut` with the default taken, `Cancelled`, `Late`) is the
//! authoritative one and reaches the receipt.
//!
//! # Auto-approval
//!
//! A run whose `[run.execution] approval` is `auto` answers every question
//! at once as the legacy runner's auto-approve interviewer does (`yes`,
//! the first option, or `auto-approved` text), attributed to the engine.
//! The question is still posted and completed, so the run's stream shows
//! what was decided.
//!
//! # Hook points for the read side
//!
//! The events posted here are the interim bridge to Fabro's read side.
//! Once the projection over Petri's records derives pending questions from
//! the `question` and `question_expired` records and the delivered answer,
//! the sink can become a no-op: the [`QuestionSink`] is the one seam to
//! replace. Two Fabro facts a Petri record does not carry are marked in
//! [`FabroInterviewer::reply`]: who answered (`AnswerSubmission::actor`,
//! carried on `interview.completed` for now) and the Fabro question id
//! that Petri's identity was mapped to. Both belong in a platform record
//! keyed on the same identity when that record kind exists.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use fabro_interview::{
    Answer as LegacyAnswer, AnswerSubmission, AnswerValue, AutoApproveInterviewer,
    ControlInterviewer, Interviewer as LegacyInterviewer, Question as LegacyQuestion,
};
use fabro_store::RunDatabase;
use fabro_types::{
    InterviewOption, Principal, QuestionType, ReviewTarget, ReviewTargetKind, RunId,
    SystemActorKind,
};
use fabro_workflow::event::{self as workflow_event, Event, RunEventSink};
use petri_execution::{
    CoordinatorRecord, ExecutionId, ExecutionObserver, InterviewError, InterviewReply,
    InterviewRequest, Interviewer,
};
use petri_runtime::engine::{EngineState, Event as EngineEvent, EventRecord};
use petri_runtime::steps::{Answer, Question, QuestionExpired, QuestionOption};
use tokio::runtime::Handle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

/// Whether a run answers its own questions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Approval {
    /// A person answers, through the API.
    Prompt,
    /// The engine answers at once, as `--auto-approve` does.
    Auto,
}

/// Petri's identity for one question, as the read side keys it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionIdentity {
    pub invocation_path: String,
    pub execution:       u64,
    pub firing:          u64,
    pub attempt:         u32,
    pub node:            String,
    pub occurrence:      u32,
    pub ask:             u32,
}

impl QuestionIdentity {
    fn of(request: &InterviewRequest) -> Self {
        Self {
            invocation_path: request.invocation_path.clone(),
            execution:       request.execution.raw(),
            firing:          request.firing.raw(),
            attempt:         request.attempt.raw(),
            node:            request.node.to_string(),
            occurrence:      request.occurrence,
            ask:             request.ask,
        }
    }
}

/// A question as Fabro shows it: the fields of `interview.started`.
#[derive(Clone, Debug, PartialEq)]
pub struct AskedQuestion {
    pub question_id:     String,
    pub identity:        QuestionIdentity,
    pub text:            String,
    pub stage:           String,
    pub question_type:   QuestionType,
    pub options:         Vec<InterviewOption>,
    pub allow_freeform:  bool,
    pub timeout_seconds: Option<f64>,
    pub review_target:   Option<ReviewTarget>,
}

/// What the adapter tells Fabro about a question, in the order it happens.
#[derive(Clone, Debug, PartialEq)]
pub enum QuestionNotice {
    Asked(AskedQuestion),
    Answered {
        question_id: String,
        text:        String,
        /// The answer as Fabro records it: the word, the key, the keys, or
        /// the text; a sensitive answer is masked.
        answer:      String,
        actor:       Principal,
        duration_ms: u64,
    },
    Expired {
        question_id: String,
        text:        String,
        stage:       String,
        duration_ms: u64,
    },
    Interrupted {
        question_id: String,
        text:        String,
        stage:       String,
        reason:      String,
        duration_ms: u64,
    },
}

impl QuestionNotice {
    /// The run event the legacy `human` stage emits for the same fact.
    #[must_use]
    pub fn into_event(self) -> Event {
        match self {
            Self::Asked(asked) => Event::InterviewStarted {
                question_id:     asked.question_id,
                question:        asked.text,
                stage:           asked.stage,
                question_type:   asked.question_type.to_string(),
                options:         asked.options,
                allow_freeform:  asked.allow_freeform,
                timeout_seconds: asked.timeout_seconds,
                context_display: None,
                review_target:   asked.review_target,
            },
            Self::Answered {
                question_id,
                text,
                answer,
                actor,
                duration_ms,
            } => Event::InterviewCompleted {
                actor: Some(actor),
                question_id,
                question: text,
                answer,
                duration_ms,
            },
            Self::Expired {
                question_id,
                text,
                stage,
                duration_ms,
            } => Event::InterviewTimeout {
                actor: None,
                question_id,
                question: text,
                stage,
                duration_ms,
            },
            Self::Interrupted {
                question_id,
                text,
                stage,
                reason,
                duration_ms,
            } => Event::InterviewInterrupted {
                actor: None,
                question_id,
                question: text,
                stage,
                reason,
                duration_ms,
            },
        }
    }

    fn question_id(&self) -> &str {
        match self {
            Self::Asked(asked) => &asked.question_id,
            Self::Answered { question_id, .. }
            | Self::Expired { question_id, .. }
            | Self::Interrupted { question_id, .. } => question_id,
        }
    }
}

/// Where the adapter posts what happens to a question: the run's event
/// stream, whichever way the process reaches it.
#[async_trait::async_trait]
pub trait QuestionSink: Send + Sync {
    async fn post(&self, notice: QuestionNotice) -> anyhow::Result<()>;
}

/// The worker's sink: the run event sink its lifecycle events go through.
pub struct EventSinkQuestions {
    sink:   RunEventSink,
    run_id: RunId,
}

impl EventSinkQuestions {
    #[must_use]
    pub fn new(sink: RunEventSink, run_id: RunId) -> Self {
        Self { sink, run_id }
    }
}

#[async_trait::async_trait]
impl QuestionSink for EventSinkQuestions {
    async fn post(&self, notice: QuestionNotice) -> anyhow::Result<()> {
        workflow_event::append_event_to_sink(&self.sink, &self.run_id, &notice.into_event())
            .await
            .map_err(anyhow::Error::new)
    }
}

/// The server's sink for a run in its own process: the run's database.
pub struct DatabaseQuestions {
    store:  RunDatabase,
    run_id: RunId,
}

impl DatabaseQuestions {
    #[must_use]
    pub fn new(store: RunDatabase, run_id: RunId) -> Self {
        Self { store, run_id }
    }
}

#[async_trait::async_trait]
impl QuestionSink for DatabaseQuestions {
    async fn post(&self, notice: QuestionNotice) -> anyhow::Result<()> {
        workflow_event::append_event(&self.store, &self.run_id, &notice.into_event()).await
    }
}

/// The questions whose expiry the gate reported, by execution and Petri
/// question id: an observer the run registers ahead of the dispatcher.
#[derive(Default)]
pub struct Expiries {
    expired: Mutex<HashSet<(ExecutionId, String)>>,
}

impl Expiries {
    fn contains(&self, execution: ExecutionId, question: &str) -> bool {
        self.expired
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&(execution, question.to_string()))
    }
}

impl ExecutionObserver for Expiries {
    fn on_engine_record(
        &self,
        execution: ExecutionId,
        record: &EventRecord,
        _recorded_at: u64,
        _state: &EngineState,
    ) {
        let EngineEvent::StepProgressRecorded { ev, .. } = &record.event else {
            return;
        };
        if let Some(expired) = QuestionExpired::from_event(ev) {
            self.expired
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert((execution, expired.question));
        }
    }

    fn on_lifecycle(&self, _record: &CoordinatorRecord) {}
}

/// The interviewer a Fabro run installs.
pub struct FabroInterviewer {
    answers:  Arc<ControlInterviewer>,
    sink:     Arc<dyn QuestionSink>,
    approval: Approval,
    expiries: Arc<Expiries>,
}

impl FabroInterviewer {
    /// Over the control interviewer the run's answers are delivered to,
    /// and the sink its questions are posted through.
    #[must_use]
    pub fn new(
        answers: Arc<ControlInterviewer>,
        sink: Arc<dyn QuestionSink>,
        approval: Approval,
    ) -> Self {
        Self {
            answers,
            sink,
            approval,
            expiries: Arc::new(Expiries::default()),
        }
    }

    /// The observer that sees a gate report a question's expiry. A run
    /// registers it ahead of the interview dispatcher, so the adapter
    /// tells an expiry from an interruption when the dispatcher ends its
    /// wait.
    #[must_use]
    pub fn observer(&self) -> Arc<dyn ExecutionObserver> {
        self.expiries.clone()
    }

    /// Post a notice; a failure after the question was asked is logged,
    /// since the answer, not the notice, is what the run depends on.
    async fn post(&self, notice: QuestionNotice) {
        let question_id = notice.question_id().to_string();
        if let Err(error) = self.sink.post(notice).await {
            warn!(
                question_id = %question_id,
                error = format!("{error:#}"),
                "a question notice could not be posted"
            );
        }
    }
}

#[async_trait::async_trait]
impl Interviewer for FabroInterviewer {
    async fn reply(&self, request: InterviewRequest, cancel: CancellationToken) -> InterviewReply {
        let asked = asked_question(&request);
        let question_id = asked.question_id.clone();
        let text = asked.text.clone();
        let stage = asked.stage.clone();
        let legacy = legacy_question(&asked);
        // HOOK POINT (read side): the mapping from Petri's identity
        // (`asked.identity`) to Fabro's question id is a platform fact
        // worth a record keyed on that identity; today it lives only in
        // the `interview.started` event posted here.
        if let Err(error) = self.sink.post(QuestionNotice::Asked(asked)).await {
            return InterviewReply::Failed(InterviewError::with_source(
                format!("question `{question_id}` could not be published to Fabro"),
                AnyhowError(error),
            ));
        }
        let mut outstanding = Outstanding {
            sink:        Arc::clone(&self.sink),
            expiries:    Arc::clone(&self.expiries),
            execution:   request.execution,
            question:    request.question.id.clone(),
            question_id: question_id.clone(),
            text:        text.clone(),
            stage:       stage.clone(),
            started:     Instant::now(),
            open:        true,
        };
        let submission = match self.approval {
            Approval::Auto => Some(AutoApproveInterviewer::engine().ask(legacy).await),
            Approval::Prompt => tokio::select! {
                submission = self.answers.ask(legacy) => Some(submission),
                () = cancel.cancelled() => None,
            },
        };
        let duration_ms = millis(outstanding.started.elapsed());
        let Some(submission) = submission else {
            // The dispatcher ended the wait: the gate expired the question,
            // the firing finished, the run was cancelled, or the run ended.
            outstanding.close_unanswered("cancelled");
            return InterviewReply::Cancelled;
        };
        // HOOK POINT (read side): `submission.actor` is who answered, a
        // Fabro fact Petri's answer record does not carry; it rides on
        // `interview.completed` until a platform record holds it.
        let Some(answer) = petri_answer(&submission.answer, &request.question) else {
            outstanding.close_unanswered(&reason_of(&submission.answer.value));
            return InterviewReply::Cancelled;
        };
        debug!(question_id = %question_id, actor = ?submission.actor, "question answered");
        outstanding.open = false;
        self.post(QuestionNotice::Answered {
            question_id,
            text,
            answer: describe(&answer, &request.question),
            actor: submission.actor,
            duration_ms,
        })
        .await;
        InterviewReply::Answered(answer)
    }
}

/// A question the adapter is waiting on. When the wait ends without an
/// answer, whether the adapter saw the cancel or the dispatcher dropped
/// the reply future first, the end of the question is posted from here
/// on its own task: `interview.timeout` when the gate reported the
/// expiry, else `interview.interrupted`.
struct Outstanding {
    sink:        Arc<dyn QuestionSink>,
    expiries:    Arc<Expiries>,
    execution:   ExecutionId,
    /// Petri's question id, as the expiry report names it.
    question:    String,
    question_id: String,
    text:        String,
    stage:       String,
    started:     Instant,
    open:        bool,
}

impl Outstanding {
    /// End the question without an answer, for `reason` unless the gate
    /// reported the expiry.
    fn close_unanswered(&mut self, reason: &str) {
        if !self.open {
            return;
        }
        self.open = false;
        let duration_ms = millis(self.started.elapsed());
        let expired = self.expiries.contains(self.execution, &self.question);
        let notice = if expired {
            QuestionNotice::Expired {
                question_id: self.question_id.clone(),
                text: self.text.clone(),
                stage: self.stage.clone(),
                duration_ms,
            }
        } else {
            QuestionNotice::Interrupted {
                question_id: self.question_id.clone(),
                text: self.text.clone(),
                stage: self.stage.clone(),
                reason: reason.to_string(),
                duration_ms,
            }
        };
        let sink = Arc::clone(&self.sink);
        let question_id = self.question_id.clone();
        let post = async move {
            if let Err(error) = sink.post(notice).await {
                warn!(
                    question_id = %question_id,
                    error = format!("{error:#}"),
                    "the end of a question could not be posted"
                );
            }
        };
        if let Ok(handle) = Handle::try_current() {
            handle.spawn(post);
        } else {
            warn!(
                question_id = %self.question_id,
                "no runtime to post the end of a question from"
            );
        }
    }
}

impl Drop for Outstanding {
    fn drop(&mut self) {
        self.close_unanswered("cancelled");
    }
}

/// An `anyhow` error as a source for Petri's interview error.
#[derive(Debug)]
struct AnyhowError(anyhow::Error);

impl std::fmt::Display for AnyhowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:#}", self.0)
    }
}

impl std::error::Error for AnyhowError {}

/// Fabro's id for a question, from Petri's identity: the node, then the
/// execution and firing (unique in the run), the occurrence and the ask
/// (a re-asked question is a new one). Only URL-safe characters, so the
/// id travels in the answer endpoint's path as it is.
#[must_use]
pub fn question_id(identity: &QuestionIdentity) -> String {
    let node: String = identity
        .node
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "{node}.x{}.f{}.q{}.a{}",
        identity.execution, identity.firing, identity.occurrence, identity.ask
    )
}

/// The question as Fabro shows it.
fn asked_question(request: &InterviewRequest) -> AskedQuestion {
    let identity = QuestionIdentity::of(request);
    let question = &request.question;
    AskedQuestion {
        question_id: question_id(&identity),
        identity,
        text: question.text.clone(),
        stage: request.node.to_string(),
        question_type: question_type(question),
        options: question
            .options
            .iter()
            .map(|option| InterviewOption {
                key:         option.key.clone(),
                label:       option.label.clone(),
                description: None,
                preview:     None,
            })
            .collect(),
        allow_freeform: question.freeform,
        timeout_seconds: question
            .timeout_ms
            .map(|ms| Duration::from_millis(ms).as_secs_f64()),
        review_target: question.reference.as_ref().and_then(|reference| {
            let kind = match reference.kind.as_deref() {
                None | Some("document") => ReviewTargetKind::Document,
                Some(other) => {
                    warn!(
                        kind = other,
                        "review target kind is not one Fabro shows; showing a document"
                    );
                    ReviewTargetKind::Document
                }
            };
            ReviewTarget::new(&reference.label, &reference.url, kind)
                .inspect_err(|error| {
                    warn!(error = %error, "review target could not be shown");
                })
                .ok()
        }),
    }
}

/// Fabro's question type: the one the gate names, else what the shape
/// implies.
fn question_type(question: &Question) -> QuestionType {
    question
        .kind
        .as_deref()
        .and_then(|kind| kind.parse().ok())
        .unwrap_or(if question.options.is_empty() {
            QuestionType::Freeform
        } else {
            QuestionType::MultipleChoice
        })
}

/// The legacy question the control interviewer waits under: only the id
/// matters to it; the rest is what the auto-approve interviewer decides on.
fn legacy_question(asked: &AskedQuestion) -> LegacyQuestion {
    let mut question = LegacyQuestion::new(asked.text.clone(), asked.question_type);
    question.id.clone_from(&asked.question_id);
    question.options.clone_from(&asked.options);
    question.allow_freeform = asked.allow_freeform;
    question.timeout_seconds = asked.timeout_seconds;
    question.stage.clone_from(&asked.stage);
    question.review_target.clone_from(&asked.review_target);
    question
}

/// Petri's answer for a legacy one, or `None` when the person or the
/// engine ended the interview without one.
fn petri_answer(answer: &LegacyAnswer, question: &Question) -> Option<Answer> {
    match &answer.value {
        AnswerValue::Yes => Some(Answer::choice(&affirmative_key(question))),
        AnswerValue::No => Some(Answer::choice(&negative_key(question))),
        AnswerValue::Selected(key) => Some(Answer::choice(key)),
        AnswerValue::MultiSelected(keys) => Some(Answer::choices(keys.iter().cloned())),
        AnswerValue::Text(text) => Some(Answer::text(text.clone())),
        AnswerValue::Cancelled
        | AnswerValue::Interrupted
        | AnswerValue::Skipped
        | AnswerValue::Timeout => None,
    }
}

/// Whether a choice is the affirmative one of a yes/no gate, as the gate
/// itself matches a `yes` answer: key `y` or `yes`, or label `yes`.
fn is_affirmative(option: &QuestionOption) -> bool {
    option.key.eq_ignore_ascii_case("y")
        || option.key.eq_ignore_ascii_case("yes")
        || strip_accelerator(&option.label).eq_ignore_ascii_case("yes")
}

fn is_negative(option: &QuestionOption) -> bool {
    option.key.eq_ignore_ascii_case("n")
        || option.key.eq_ignore_ascii_case("no")
        || strip_accelerator(&option.label).eq_ignore_ascii_case("no")
}

/// The key a `yes` answer names: the affirmative choice, else the word
/// itself for the gate to match.
fn affirmative_key(question: &Question) -> String {
    question
        .options
        .iter()
        .find(|option| is_affirmative(option))
        .map_or_else(|| "yes".to_string(), |option| option.key.clone())
}

/// The key a `no` answer names: the negative choice, else the first choice
/// that is not affirmative, else the word itself.
fn negative_key(question: &Question) -> String {
    question
        .options
        .iter()
        .find(|option| is_negative(option))
        .or_else(|| {
            question
                .options
                .iter()
                .find(|option| !is_affirmative(option))
        })
        .map_or_else(|| "no".to_string(), |option| option.key.clone())
}

/// A label without its `[K] ` accelerator prefix.
fn strip_accelerator(label: &str) -> &str {
    let trimmed = label.trim();
    match trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
    {
        Some((_, rest)) => rest.trim(),
        None => trimmed,
    }
}

/// The answer as `interview.completed` records it. A sensitive text
/// answer is never written out: the dispatcher registers it as a secret.
fn describe(answer: &Answer, question: &Question) -> String {
    if !answer.choices.is_empty() {
        return answer.choices.join(", ");
    }
    if let Some(choice) = &answer.choice {
        return choice.clone();
    }
    match &answer.text {
        Some(_) if question.sensitive => "***".to_string(),
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

fn reason_of(value: &AnswerValue) -> String {
    match value {
        AnswerValue::Cancelled => "cancelled",
        AnswerValue::Interrupted => "interrupted",
        AnswerValue::Skipped => "skipped",
        AnswerValue::Timeout => "timeout",
        _ => "unanswered",
    }
    .to_string()
}

fn millis(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

/// An engine actor, for callers that answer on the run's behalf.
#[must_use]
pub fn engine_actor() -> Principal {
    Principal::System {
        system_kind: SystemActorKind::Engine,
    }
}

/// A submission on the run's behalf.
#[must_use]
pub fn engine_submission(answer: LegacyAnswer) -> AnswerSubmission {
    AnswerSubmission::new(answer, engine_actor())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yes_no() -> Question {
        let mut question = Question::new("gate#3", "Go?");
        question.options = vec![
            QuestionOption {
                key:   "Y".into(),
                label: "[Y] Yes".into(),
            },
            QuestionOption {
                key:   "N".into(),
                label: "[N] No".into(),
            },
        ];
        question.kind = Some("yes_no".into());
        question
    }

    #[test]
    fn a_question_id_is_url_safe_and_names_the_identity() {
        let identity = QuestionIdentity {
            invocation_path: "/branch:fan@2:0:a".into(),
            execution:       2,
            firing:          3,
            attempt:         1,
            node:            "approve plan".into(),
            occurrence:      1,
            ask:             2,
        };
        assert_eq!(question_id(&identity), "approve_plan.x2.f3.q1.a2");
    }

    #[test]
    fn yes_and_no_name_the_gates_choices_by_key() {
        let question = yes_no();
        assert_eq!(
            petri_answer(&LegacyAnswer::yes(), &question),
            Some(Answer::choice("Y"))
        );
        assert_eq!(
            petri_answer(&LegacyAnswer::no(), &question),
            Some(Answer::choice("N"))
        );
        let mut approve = Question::new("q", "Ship?");
        approve.options = vec![
            QuestionOption {
                key:   "A".into(),
                label: "Approve".into(),
            },
            QuestionOption {
                key:   "R".into(),
                label: "Reject".into(),
            },
        ];
        assert_eq!(
            petri_answer(&LegacyAnswer::yes(), &approve),
            Some(Answer::choice("yes")),
            "no affirmative choice: the word reaches the gate to match"
        );
        assert_eq!(
            petri_answer(&LegacyAnswer::no(), &approve),
            Some(Answer::choice("A")),
            "the first choice that is not affirmative"
        );
    }

    #[test]
    fn selections_text_and_refusals_map_to_petris_shapes() {
        let question = yes_no();
        assert_eq!(
            petri_answer(
                &LegacyAnswer {
                    value:           AnswerValue::Selected("N".into()),
                    selected_option: None,
                    text:            None,
                },
                &question
            ),
            Some(Answer::choice("N"))
        );
        assert_eq!(
            petri_answer(
                &LegacyAnswer::multi_selected(vec!["A".into(), "B".into()]),
                &question
            ),
            Some(Answer::choices(["A", "B"]))
        );
        assert_eq!(
            petri_answer(&LegacyAnswer::text("ship it"), &question),
            Some(Answer::text("ship it"))
        );
        for ended in [
            LegacyAnswer::cancelled(),
            LegacyAnswer::interrupted(),
            LegacyAnswer::skipped(),
            LegacyAnswer::timeout(),
        ] {
            assert_eq!(petri_answer(&ended, &question), None);
        }
    }

    #[test]
    fn the_question_type_is_the_gates_else_the_shapes() {
        assert_eq!(question_type(&yes_no()), QuestionType::YesNo);
        let mut choice = yes_no();
        choice.kind = None;
        assert_eq!(question_type(&choice), QuestionType::MultipleChoice);
        let mut free = Question::new("q", "Name?");
        free.freeform = true;
        assert_eq!(question_type(&free), QuestionType::Freeform);
    }

    #[test]
    fn a_sensitive_text_answer_is_described_masked() {
        let mut question = Question::new("q", "Token?");
        question.sensitive = true;
        assert_eq!(describe(&Answer::text("hunter2"), &question), "***");
        question.sensitive = false;
        assert_eq!(describe(&Answer::text("hunter2"), &question), "hunter2");
        assert_eq!(describe(&Answer::choices(["A", "B"]), &question), "A, B");
    }
}
