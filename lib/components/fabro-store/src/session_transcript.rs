//! The server-side transcript fold (ADR-0046 phase 2,
//! denkhaus/work#1679): a session's recorded `run.session.*` events
//! folded into user and assistant messages, with the tool calls counted
//! instead of carried and a token budget applied.
//!
//! The fold ports the frontend replay (`apps/fabro-web`
//! `ask-transcript.ts`) to the server: the streaming `assistant_delta`
//! frames are skipped because the `assistant_message` event carries the
//! turn's full text, a failed or interrupted turn lands as a visible
//! assistant note, and a tool call never becomes a message — it is
//! summarized in the transcript's tool-call counts. On top of the fold
//! sits the budget cap: messages are included whole, in order, until the
//! cap (estimated at four characters per token) is exhausted; everything
//! past it is dropped and counted.

use chrono::{DateTime, Utc};
use fabro_types::session_event::{SessionToolCallCompletedProps, SessionTurnFailedProps};
use fabro_types::{
    RunId, SessionEvent, SessionEventBody, SessionId, SessionTranscript, SessionTranscriptBudget,
    SessionTranscriptMessage, SessionTranscriptRole, SessionTranscriptToolCalls,
};

/// The token estimate: four characters per token, rounded up. An
/// approximation is the point — the budget bounds context size, it does
/// not bill tokens.
#[must_use]
pub fn estimate_tokens(text: &str) -> u64 {
    let chars = text.chars().count() as u64;
    chars.div_ceil(4)
}

/// The default transcript budget the doors apply when no explicit
/// `max_tokens` arrives.
pub const DEFAULT_MAX_TOKENS: u64 = 4_000;

/// The budget's ceiling; a request past it is clamped.
pub const MAX_MAX_TOKENS: u64 = 50_000;

/// The transcript's event-load ceiling, the frontend replay's bound: at
/// most this many events are read before the fold runs.
pub const TRANSCRIPT_MAX_EVENTS: usize = 10_000;

/// Fold `session_id`'s events into a compacted transcript under
/// `max_tokens`. `run_id` and `title` carry the session's anchor and its
/// creation title (both from the session projection).
#[must_use]
pub fn fold_session_transcript(
    session_id: SessionId,
    run_id: Option<RunId>,
    title: Option<String>,
    events: &[SessionEvent],
    max_tokens: u64,
) -> SessionTranscript {
    let folded = fold_messages_and_tool_calls(events);
    let (messages, budget) = apply_budget(folded.messages, folded.trailing_tool_calls, max_tokens);
    SessionTranscript {
        session_id,
        run_id,
        title,
        messages,
        tool_calls: folded.tool_calls,
        budget,
    }
}

/// One folded message plus the tool calls recorded inside its window —
/// the count the budget report needs when the message drops off the end.
struct FoldedMessage {
    message:    SessionTranscriptMessage,
    tool_calls: u64,
}

/// The assistant text accumulated since the last flush, anchored at the
/// event that opened it.
struct AssistantSegment {
    text:       Option<String>,
    seq:        u32,
    ts:         Option<DateTime<Utc>>,
    tool_calls: u64,
}

impl AssistantSegment {
    fn new() -> Self {
        Self {
            text:       None,
            seq:        0,
            ts:         None,
            tool_calls: 0,
        }
    }

    /// Append one completed assistant text segment; consecutive segments
    /// (text, tool call, text) join into one compacted message, the
    /// compacted shape of the frontend's multi-part assistant message.
    fn append_text(&mut self, text: &str, seq: u32, ts: DateTime<Utc>) {
        if self.text.is_none() {
            self.seq = seq;
            self.ts = Some(ts);
            self.text = Some(String::new());
        } else {
            let joined = self.text.get_or_insert(String::new());
            joined.push_str("\n\n");
        }
        self.text
            .as_mut()
            .expect("the segment just opened or joined")
            .push_str(text);
    }

    /// Close the segment onto `messages`. Calls recorded under a segment
    /// that produced no text (a tool-only turn tail) come back as the
    /// return value — the budget report counts them with the transcript
    /// tail they close.
    fn flush(&mut self, messages: &mut Vec<FoldedMessage>) -> u64 {
        if let Some(text) = self.text.take() {
            messages.push(FoldedMessage {
                message:    SessionTranscriptMessage {
                    role: SessionTranscriptRole::Assistant,
                    text,
                    seq: self.seq,
                    ts: self.ts.expect("an opened segment has a timestamp"),
                },
                tool_calls: self.tool_calls,
            });
            self.tool_calls = 0;
        }
        std::mem::take(&mut self.tool_calls)
    }
}

struct FoldedTranscript {
    messages:            Vec<FoldedMessage>,
    /// Tool calls recorded after the last message window closed — the
    /// calls of a tool-only turn tail with no assistant text. The budget
    /// counts them with the transcript tail when anything is dropped.
    trailing_tool_calls: u64,
    tool_calls:          SessionTranscriptToolCalls,
}

/// The raw fold: every message the events describe, uncapped, with each
/// message carrying the tool calls observed in its window.
fn fold_messages_and_tool_calls(events: &[SessionEvent]) -> FoldedTranscript {
    let mut messages: Vec<FoldedMessage> = Vec::new();
    let mut tool_calls = SessionTranscriptToolCalls::new();
    let mut segment = AssistantSegment::new();
    let mut trailing_tool_calls = 0u64;

    for event in events {
        match &event.body {
            SessionEventBody::UserMessage(props) => {
                trailing_tool_calls += segment.flush(&mut messages);
                if !props.text.is_empty() {
                    messages.push(FoldedMessage {
                        message:    SessionTranscriptMessage {
                            role: SessionTranscriptRole::User,
                            text: props.text.clone(),
                            seq:  event.seq,
                            ts:   event.ts,
                        },
                        tool_calls: 0,
                    });
                }
            }
            SessionEventBody::AssistantMessage(props) => {
                // An empty text marks nothing: the frontend fold drops it
                // the same way.
                if props.text.is_empty() {
                    continue;
                }
                segment.append_text(&props.text, event.seq, event.ts);
            }
            SessionEventBody::ToolCallStarted(props) => {
                tool_calls.total += 1;
                *tool_calls
                    .by_tool
                    .entry(props.tool_name.clone())
                    .or_insert(0) += 1;
                segment.tool_calls += 1;
            }
            SessionEventBody::ToolCallCompleted(props) => {
                // The started frame already counted the call; a completion
                // only moves the error count when it failed.
                record_completed_error(&mut tool_calls, props);
            }
            SessionEventBody::TurnFailed(props) => {
                // A failed turn must be VISIBLE (work#1681): the error text
                // lands as its own assistant note, with the retryable flag
                // riding the visible text.
                trailing_tool_calls += segment.flush(&mut messages);
                messages.push(FoldedMessage {
                    message:    SessionTranscriptMessage {
                        role: SessionTranscriptRole::Assistant,
                        text: turn_failed_text(props),
                        seq:  event.seq,
                        ts:   event.ts,
                    },
                    tool_calls: 0,
                });
            }
            SessionEventBody::TurnInterrupted(_) => {
                trailing_tool_calls += segment.flush(&mut messages);
                messages.push(FoldedMessage {
                    message:    SessionTranscriptMessage {
                        role: SessionTranscriptRole::Assistant,
                        text: "Turn interrupted.".to_string(),
                        seq:  event.seq,
                        ts:   event.ts,
                    },
                    tool_calls: 0,
                });
            }
            // `turn.succeeded` closes the open assistant message, exactly
            // the frontend fold's flush. The delta frames are streaming
            // fragments: the assistant_message event carries the turn's
            // full text, so replaying deltas would duplicate it — the
            // frontend fold's rule for skipping them. Creation, capability
            // changes, and turn starts contribute nothing: the session
            // summary holds them.
            SessionEventBody::TurnSucceeded(_) => {
                trailing_tool_calls += segment.flush(&mut messages);
            }
            SessionEventBody::AssistantDelta(_)
            | SessionEventBody::Created(_)
            | SessionEventBody::CapabilityChanged(_)
            | SessionEventBody::TurnStarted(_) => {}
        }
    }
    trailing_tool_calls += segment.flush(&mut messages);

    FoldedTranscript {
        messages,
        trailing_tool_calls,
        tool_calls,
    }
}

fn turn_failed_text(props: &SessionTurnFailedProps) -> String {
    if props.retryable {
        format!("Turn failed: {} (retryable)", props.error)
    } else {
        format!("Turn failed: {}", props.error)
    }
}

fn record_completed_error(
    tool_calls: &mut SessionTranscriptToolCalls,
    props: &SessionToolCallCompletedProps,
) {
    if props.is_error {
        tool_calls.errors += 1;
    }
}

/// Apply the budget: messages in full, in order, while the cap holds.
/// The first message is always kept — a budget smaller than the opening
/// message still returns a useful transcript instead of nothing — and
/// everything past the cap is dropped and counted. `trailing_tool_calls`
/// are the calls of a tool-only tail with no message to ride; when
/// anything was dropped, the whole tail went with it.
fn apply_budget(
    messages: Vec<FoldedMessage>,
    trailing_tool_calls: u64,
    max_tokens: u64,
) -> (Vec<SessionTranscriptMessage>, SessionTranscriptBudget) {
    let mut kept: Vec<SessionTranscriptMessage> = Vec::new();
    let mut used_tokens = 0u64;
    let mut truncated = false;
    let mut omitted_messages = 0u64;
    let mut omitted_tool_calls = 0u64;
    let mut first = true;
    for folded in messages {
        let cost = estimate_tokens(&folded.message.text);
        if first || used_tokens + cost <= max_tokens {
            used_tokens += cost;
            kept.push(folded.message);
            first = false;
        } else {
            truncated = true;
            omitted_messages += 1;
            omitted_tool_calls += folded.tool_calls;
        }
    }
    if truncated {
        omitted_tool_calls += trailing_tool_calls;
    }
    (kept, SessionTranscriptBudget {
        max_tokens,
        used_tokens,
        truncated,
        omitted_messages,
        omitted_tool_calls,
    })
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use fabro_types::session_event::{
        SessionAssistantDeltaProps, SessionAssistantMessageProps, SessionCapabilityChangedProps,
        SessionCreatedProps, SessionToolCallStartedProps, SessionTurnFailedCode,
        SessionTurnInterruptedProps, SessionTurnStartedProps, SessionTurnSucceededProps,
        SessionUserMessageProps,
    };
    use fabro_types::{AskFabroCapability, SessionEvent, TurnId};

    use super::*;

    fn at(seconds: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 25, 12, 0, seconds).unwrap()
    }

    fn event(seq: u32, body: SessionEventBody) -> SessionEvent {
        SessionEvent {
            seq,
            session_id: SessionId::new(),
            run_id: None,
            ts: at(seq),
            body,
        }
    }

    fn created() -> SessionEventBody {
        SessionEventBody::Created(SessionCreatedProps {
            title:      Some("Ask Fabro".to_string()),
            model:      Some("test-model".to_string()),
            provider:   None,
            capability: AskFabroCapability::ReadOnly,
        })
    }

    fn user_message(seq: u32, turn_id: TurnId, text: &str) -> SessionEvent {
        event(
            seq,
            SessionEventBody::UserMessage(SessionUserMessageProps {
                turn_id,
                text: text.to_string(),
            }),
        )
    }

    fn assistant_delta(seq: u32, turn_id: TurnId, delta: &str) -> SessionEvent {
        event(
            seq,
            SessionEventBody::AssistantDelta(SessionAssistantDeltaProps {
                turn_id,
                delta: delta.to_string(),
            }),
        )
    }

    fn assistant_message(seq: u32, turn_id: TurnId, text: &str) -> SessionEvent {
        event(
            seq,
            SessionEventBody::AssistantMessage(SessionAssistantMessageProps {
                turn_id,
                text: text.to_string(),
                model: Some("test-model".to_string()),
                usage: serde_json::json!({ "output_tokens": 4 }),
            }),
        )
    }

    fn tool_call_started(seq: u32, turn_id: TurnId, name: &str) -> SessionEvent {
        event(
            seq,
            SessionEventBody::ToolCallStarted(SessionToolCallStartedProps {
                turn_id,
                tool_name: name.to_string(),
                tool_call_id: format!("call_{seq}"),
                arguments: serde_json::json!({}),
            }),
        )
    }

    fn tool_call_completed(seq: u32, turn_id: TurnId, is_error: bool) -> SessionEvent {
        event(
            seq,
            SessionEventBody::ToolCallCompleted(SessionToolCallCompletedProps {
                turn_id,
                tool_name: "read_file".to_string(),
                tool_call_id: format!("call_{}", seq - 1),
                output: serde_json::json!("bytes"),
                is_error,
                output_bytes_observed: None,
                output_bytes_retained: None,
                output_bytes_omitted: None,
            }),
        )
    }

    fn turn_succeeded(seq: u32, turn_id: TurnId) -> SessionEvent {
        event(
            seq,
            SessionEventBody::TurnSucceeded(SessionTurnSucceededProps {
                turn_id,
                output: None,
            }),
        )
    }

    fn turn_failed(seq: u32, turn_id: TurnId, error: &str, retryable: bool) -> SessionEvent {
        event(
            seq,
            SessionEventBody::TurnFailed(SessionTurnFailedProps {
                turn_id,
                error: error.to_string(),
                output: None,
                code: SessionTurnFailedCode::AgentError,
                retryable,
            }),
        )
    }

    fn turn_interrupted(seq: u32, turn_id: TurnId) -> SessionEvent {
        event(
            seq,
            SessionEventBody::TurnInterrupted(SessionTurnInterruptedProps {
                turn_id,
                error: Some("Interrupted.".to_string()),
            }),
        )
    }

    fn fold(events: &[SessionEvent]) -> SessionTranscript {
        fold_session_transcript(
            SessionId::new(),
            None,
            Some("Ask Fabro".to_string()),
            events,
            DEFAULT_MAX_TOKENS,
        )
    }

    /// The frontend fold's first case: recorded events become user and
    /// assistant messages; the delta frame is skipped because the
    /// assistant_message event carries the turn's full text.
    #[test]
    fn folds_recorded_events_into_user_and_assistant_messages() {
        let turn = TurnId::new();
        let transcript = fold(&[
            event(1, created()),
            user_message(2, turn, "What happened?"),
            assistant_delta(3, turn, "ignored"),
            assistant_message(4, turn, "All good."),
            turn_succeeded(5, turn),
        ]);

        assert_eq!(transcript.messages.len(), 2);
        assert_eq!(transcript.messages[0].role, SessionTranscriptRole::User);
        assert_eq!(transcript.messages[0].text, "What happened?");
        assert_eq!(transcript.messages[0].seq, 2);
        assert_eq!(
            transcript.messages[1].role,
            SessionTranscriptRole::Assistant
        );
        assert_eq!(transcript.messages[1].text, "All good.");
        assert!(!transcript.budget.truncated);
    }

    /// Tool calls never become messages; they are counted, and errors move
    /// the error count (the completion's is_error is the verdict).
    #[test]
    fn keeps_tool_calls_as_counts_not_messages() {
        let turn = TurnId::new();
        let transcript = fold(&[
            user_message(1, turn, "check"),
            tool_call_started(2, turn, "fabro_run_events"),
            tool_call_completed(3, turn, false),
            tool_call_started(4, turn, "read_file"),
            tool_call_completed(5, turn, true),
            assistant_message(6, turn, "Done."),
            turn_succeeded(7, turn),
        ]);

        assert_eq!(transcript.messages.len(), 2, "user + assistant only");
        assert_eq!(transcript.messages[1].text, "Done.");
        assert_eq!(transcript.tool_calls.total, 2);
        assert_eq!(transcript.tool_calls.errors, 1);
        assert_eq!(
            transcript.tool_calls.by_tool.get("fabro_run_events"),
            Some(&1)
        );
        assert_eq!(transcript.tool_calls.by_tool.get("read_file"), Some(&1));
        assert_eq!(transcript.budget.omitted_tool_calls, 0);
    }

    /// A failed turn folds to a visible failure note (work#1681), with the
    /// retryable flag riding the visible text.
    #[test]
    fn a_failed_turn_folds_to_a_visible_failure_note() {
        let turn = TurnId::new();
        let transcript = fold(&[
            user_message(1, turn, "Say hello."),
            turn_failed(2, turn, "run sandbox was not created", false),
        ]);

        assert_eq!(transcript.messages.len(), 2);
        assert_eq!(
            transcript.messages[1].role,
            SessionTranscriptRole::Assistant
        );
        assert_eq!(
            transcript.messages[1].text,
            "Turn failed: run sandbox was not created"
        );

        let retryable = fold(&[
            user_message(1, turn, "Say hello."),
            turn_failed(2, turn, "rate limited", true),
        ]);
        assert_eq!(
            retryable.messages[1].text,
            "Turn failed: rate limited (retryable)"
        );
    }

    /// An interrupted turn folds to a visible note (work#1681).
    #[test]
    fn an_interrupted_turn_folds_to_a_visible_note() {
        let turn = TurnId::new();
        let transcript = fold(&[user_message(1, turn, "Stop."), turn_interrupted(2, turn)]);

        assert_eq!(transcript.messages.len(), 2);
        assert_eq!(transcript.messages[1].text, "Turn interrupted.");
    }

    /// An empty event log folds to an empty transcript.
    #[test]
    fn an_empty_event_log_folds_to_an_empty_transcript() {
        let transcript = fold(&[]);
        assert!(transcript.messages.is_empty());
        assert!(!transcript.budget.truncated);
    }

    /// The budget includes whole messages while the cap holds, drops the
    /// tail, and reports the omissions.
    #[test]
    fn the_budget_includes_whole_messages_until_the_cap() {
        let turn = TurnId::new();
        // 40-char text = 10 tokens per message.
        let text = "x".repeat(40);
        let events = vec![
            user_message(1, turn, &text),
            assistant_message(2, turn, &text),
            turn_succeeded(3, turn),
            user_message(4, turn, &text),
            assistant_message(5, turn, &text),
            turn_succeeded(6, turn),
        ];
        let transcript = fold_session_transcript(SessionId::new(), None, None, &events, 25);

        assert_eq!(
            transcript.messages.len(),
            2,
            "two messages fit a 25-token budget"
        );
        assert!(transcript.budget.truncated);
        assert_eq!(transcript.budget.used_tokens, 20);
        assert_eq!(transcript.budget.omitted_messages, 2);
        assert_eq!(transcript.budget.max_tokens, 25);
    }

    /// A budget smaller than the opening message still keeps it: a capped
    /// transcript is never empty while the session has any message.
    #[test]
    fn a_budget_smaller_than_the_first_message_still_keeps_it() {
        let turn = TurnId::new();
        let events = vec![
            user_message(1, turn, &"y".repeat(400)),
            assistant_message(2, turn, &"z".repeat(400)),
            turn_succeeded(3, turn),
        ];
        let transcript = fold_session_transcript(SessionId::new(), None, None, &events, 10);

        assert_eq!(transcript.messages.len(), 1);
        assert!(transcript.budget.truncated);
        assert_eq!(transcript.budget.omitted_messages, 1);
        assert!(transcript.budget.used_tokens > transcript.budget.max_tokens);
    }

    /// Tool calls in the dropped tail count as omitted tool calls.
    #[test]
    fn tool_calls_in_the_dropped_tail_count_as_omitted() {
        let turn = TurnId::new();
        let events = vec![
            user_message(1, turn, &"a".repeat(400)),
            turn_succeeded(2, turn),
            user_message(3, turn, &"b".repeat(400)),
            tool_call_started(4, turn, "fabro_run_events"),
            tool_call_completed(5, turn, false),
            turn_succeeded(6, turn),
        ];
        let transcript = fold_session_transcript(SessionId::new(), None, None, &events, 10);

        assert_eq!(transcript.messages.len(), 1);
        assert_eq!(transcript.budget.omitted_messages, 1);
        assert_eq!(transcript.budget.omitted_tool_calls, 1);
    }

    /// The token estimate is four characters per token, rounded up.
    #[test]
    fn estimates_four_characters_per_token() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abc"), 1);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
    }

    /// The session metadata (anchor + title) rides the transcript; a
    /// runless session's anchor is None.
    #[test]
    fn the_transcript_carries_the_session_anchor_and_title() {
        let session_id = SessionId::new();
        let transcript = fold_session_transcript(
            session_id,
            None,
            Some("Ask Fabro".to_string()),
            &[],
            DEFAULT_MAX_TOKENS,
        );
        assert_eq!(transcript.session_id, session_id);
        assert_eq!(transcript.run_id, None);
        assert_eq!(transcript.title.as_deref(), Some("Ask Fabro"));
    }

    /// Consecutive assistant segments (text, tool call, text) fold into one
    /// assistant message whose text joins the segments — the compacted
    /// shape of the frontend's multi-part assistant message.
    #[test]
    fn consecutive_assistant_segments_join_into_one_message() {
        let turn = TurnId::new();
        let transcript = fold(&[
            user_message(1, turn, "check"),
            assistant_message(2, turn, "Looking."),
            tool_call_started(3, turn, "fabro_run_events"),
            tool_call_completed(4, turn, false),
            assistant_message(5, turn, "Found it."),
            turn_succeeded(6, turn),
        ]);

        assert_eq!(transcript.messages.len(), 2);
        assert_eq!(transcript.messages[1].text, "Looking.\n\nFound it.");
    }

    /// Creation, capability, and turn-start events contribute nothing to
    /// the message list.
    #[test]
    fn lifecycle_events_contribute_no_messages() {
        let turn = TurnId::new();
        let transcript = fold(&[
            event(1, created()),
            event(
                2,
                SessionEventBody::CapabilityChanged(SessionCapabilityChangedProps {
                    capability: AskFabroCapability::FullAccess,
                    actor:      None,
                }),
            ),
            event(
                3,
                SessionEventBody::TurnStarted(SessionTurnStartedProps {
                    turn_id: turn,
                    input:   "What happened?".to_string(),
                }),
            ),
        ]);

        assert!(transcript.messages.is_empty());
    }
}
