//! `fabro ask` — the Ask Fabro CLI door (ADR-0046, work#1453): one
//! question, one streamed answer. The turn lands in the session
//! `--session` names (a conversation to continue, UI-started or
//! CLI-started — the transcript is one) or in a session opened on a run
//! anchor: the positional selector, or — run-less, the supervisor's
//! posture — the newest live run. `--capability` performs the
//! server-enforced, audited flip the web UI performs.

use anyhow::{Result, anyhow, bail};
use fabro_api::types::CreateRunSessionRequest;
use fabro_types::{RunId, SessionEvent, SessionEventBody};

use crate::args::AskArgs;
use crate::command_context::CommandContext;
use crate::server_client::Client;
use crate::shared::print_json_pretty;

pub(crate) async fn run(args: AskArgs, base_ctx: &CommandContext) -> Result<()> {
    if args.session.is_some() && args.prompt.is_none() && args.capability.is_none() {
        bail!(
            "nothing to do on the session — pass a question (-p) to continue it, \
             or --capability to flip it"
        );
    }
    let ctx = base_ctx.with_target(&args.server)?;
    let client = ctx.server().await?;

    // The session the turn belongs to: an existing conversation via
    // --session, or one opened on the run anchor — the positional
    // selector, or the newest live run when the caller omits it (the
    // run-less supervisor posture, work#1453; the storage-level session
    // stays run-anchored per work#1468).
    let (session_id, created) = if let Some(session_id) = args.session {
        (session_id, None)
    } else {
        let prompt = args
            .prompt
            .as_deref()
            .ok_or_else(|| anyhow!("a prompt is required when opening a new session"))?;
        let run_id = resolve_run_anchor(&client, args.run.as_deref()).await?;
        let session = client
            .create_run_session(run_id, CreateRunSessionRequest {
                title:    Some(session_title(prompt)),
                model:    args.model,
                provider: None,
            })
            .await?;
        (session.id, Some(session))
    };

    let mut renderer = AskRenderer::new();
    if let Some(session) = &created {
        renderer.announce(&format!("[session] {}", session.id))?;
    }

    // The capability flip runs ahead of the turn so the posture applies
    // to it (ADR-0046): the same server-enforced, audited door the web UI
    // performs, driven by the CLI's own principal. A flip without a
    // question is the whole verb — the posture changed, no turn follows.
    if let Some(capability) = args.capability {
        let detail = client
            .set_session_capability(session_id, capability.into())
            .await?;
        if ctx.json_output() {
            print_json_pretty(&detail)?;
        } else {
            renderer.announce(&format!("[capability] {}", detail.record.capability))?;
        }
    }

    let Some(prompt) = args.prompt.as_deref() else {
        return Ok(());
    };
    let mut stream = client
        .submit_session_turn_stream(session_id, prompt)
        .await?;

    let mut terminal_error = None;
    let mut saw_terminal = false;
    while let Some(event) = stream.next_event().await? {
        renderer.render(&event, ctx.json_output())?;
        match &event.body {
            SessionEventBody::TurnSucceeded(_) | SessionEventBody::TurnInterrupted(_) => {
                saw_terminal = true;
            }
            SessionEventBody::TurnFailed(props) => {
                saw_terminal = true;
                terminal_error = Some(props.error.clone());
            }
            _ => {}
        }
    }

    if let Some(error) = terminal_error {
        bail!(error);
    }
    if !saw_terminal {
        bail!("session turn ended before a terminal event was received");
    }
    Ok(())
}

/// The run anchor for a new session: the positional selector (run ID
/// prefix or workflow name), or — run-less, the supervisor's posture
/// (work#1453) — the server's live-anchor door (work#1521): one call
/// answering "the newest run a conversation may anchor to" (`not
/// archived, not terminal`, work#1506 — a terminal run's sandbox is
/// reaped, and anchoring to it fails the first turn with
/// `sandbox_unavailable`). The predicate lives server-side; this client
/// keeps no copy.
async fn resolve_run_anchor(client: &Client, selector: Option<&str>) -> Result<RunId> {
    let Some(selector) = selector else {
        return client
            .live_anchor_run()
            .await?
            .map(|run| run.id)
            .ok_or_else(|| {
                anyhow!(
                    "no live run to anchor the session — pass a run selector, or start a run first"
                )
            });
    };
    Ok(client.resolve_run(selector).await?.id)
}

fn session_title(prompt: &str) -> String {
    const MAX_CHARS: usize = 80;
    let trimmed = prompt.trim();
    if trimmed.chars().count() <= MAX_CHARS {
        return trimmed.to_string();
    }
    let mut title = trimmed.chars().take(MAX_CHARS - 3).collect::<String>();
    title.push_str("...");
    title
}

/// Renders one session event for `fabro ask`'s human output, in the state
/// of its turn: the deltas of an answer already printed it, so the
/// assistant message that follows only ends the line — printing its text
/// again duplicated every streamed answer (fabro-bd6c). A message no delta
/// preceded (a non-streaming model, a replayed turn) prints once. Tool
/// calls and capability changes print as one-line cards, so the transcript
/// reads like the UI's turn stream.
struct AskRenderer {
    streamed:        bool,
    pending_newline: bool,
}

impl AskRenderer {
    fn new() -> Self {
        Self {
            streamed:        false,
            pending_newline: false,
        }
    }

    #[allow(
        clippy::print_stdout,
        reason = "The ask command streams assistant output and JSON events to stdout."
    )]
    fn render(&mut self, event: &SessionEvent, json_output: bool) -> Result<()> {
        self.render_to(event, json_output, &mut StdoutWriter)
    }

    #[allow(
        clippy::print_stdout,
        reason = "The ask command streams assistant output and JSON events to stdout."
    )]
    fn announce(&mut self, line: &str) -> Result<()> {
        self.announce_to(line, &mut StdoutWriter)
    }

    /// A one-line note ahead of the turn's stream: the session a caller
    /// can resume by id, the capability a flip moved the session to.
    /// Human output only; `--json` prints the structured session detail
    /// at the flip site instead.
    fn announce_to(&mut self, line: &str, out: &mut dyn std::fmt::Write) -> Result<()> {
        self.begin_line(out)?;
        writeln!(out, "{line}")?;
        Ok(())
    }

    fn render_to(
        &mut self,
        event: &SessionEvent,
        json_output: bool,
        out: &mut dyn std::fmt::Write,
    ) -> Result<()> {
        if json_output {
            writeln!(out, "{}", serde_json::to_string(event)?)?;
            return Ok(());
        }

        match &event.body {
            SessionEventBody::AssistantDelta(props) => {
                write!(out, "{}", props.delta)?;
                self.streamed = true;
                self.pending_newline = !props.delta.ends_with('\n');
            }
            SessionEventBody::AssistantMessage(props) if !props.text.is_empty() => {
                if self.streamed {
                    if self.pending_newline {
                        writeln!(out)?;
                    }
                } else {
                    writeln!(out, "{}", props.text)?;
                }
                self.streamed = false;
                self.pending_newline = false;
            }
            // A new turn resets the streaming state: its first message is
            // not the continuation of the previous answer.
            SessionEventBody::TurnStarted(_) => {
                self.streamed = false;
                self.pending_newline = false;
            }
            SessionEventBody::CapabilityChanged(props) => {
                self.begin_line(out)?;
                writeln!(out, "[capability] {}", props.capability)?;
            }
            SessionEventBody::ToolCallStarted(props) => {
                self.begin_line(out)?;
                writeln!(out, "[tool] {}", props.tool_name)?;
            }
            SessionEventBody::ToolCallCompleted(props) => {
                self.begin_line(out)?;
                writeln!(
                    out,
                    "[tool] {} {}",
                    props.tool_name,
                    if props.is_error { "failed" } else { "ok" }
                )?;
            }
            _ => {}
        }
        Ok(())
    }

    /// A card line starts on its own line: a delta stream caught mid-line
    /// ends before the note prints.
    fn begin_line(&mut self, out: &mut dyn std::fmt::Write) -> Result<()> {
        if self.pending_newline {
            writeln!(out)?;
            self.pending_newline = false;
        }
        Ok(())
    }
}

/// stdout as [`std::fmt::Write`], so the renderer stays a pure writer the
/// tests can capture. The crate's clippy config routes terminal output
/// through the print macros, so the adapter writes through `print!`
/// itself.
struct StdoutWriter;

impl std::fmt::Write for StdoutWriter {
    #[allow(
        clippy::print_stdout,
        reason = "The ask command streams assistant output and JSON events to stdout."
    )]
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        print!("{text}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use fabro_types::session_event::{
        SessionAssistantDeltaProps, SessionAssistantMessageProps, SessionCapabilityChangedProps,
        SessionToolCallCompletedProps, SessionToolCallStartedProps, SessionTurnStartedProps,
    };
    use fabro_types::{AskFabroCapability, RunId, SessionId, TurnId};

    use super::*;

    fn event(body: SessionEventBody) -> SessionEvent {
        SessionEvent {
            seq: 1,
            session_id: SessionId::new(),
            run_id: RunId::new(),
            ts: "2026-09-22T00:00:00Z".parse().unwrap(),
            body,
        }
    }

    fn delta(text: &str) -> SessionEvent {
        event(SessionEventBody::AssistantDelta(
            SessionAssistantDeltaProps {
                turn_id: TurnId::new(),
                delta:   text.to_string(),
            },
        ))
    }

    fn message(text: &str) -> SessionEvent {
        event(SessionEventBody::AssistantMessage(
            SessionAssistantMessageProps {
                turn_id: TurnId::new(),
                text:    text.to_string(),
                model:   None,
                usage:   serde_json::Value::Null,
            },
        ))
    }

    /// The streamed answer prints once: its deltas, then the assistant
    /// message only ends the line (fabro-bd6c).
    #[test]
    fn a_streamed_answer_is_not_printed_twice() {
        let mut renderer = AskRenderer::new();
        let mut out = String::new();
        for event in [delta("Hello"), delta(" world"), message("Hello world")] {
            renderer.render_to(&event, false, &mut out).unwrap();
        }
        assert_eq!(out, "Hello world\n");
    }

    /// A message no delta preceded prints its text once — the
    /// non-streaming and replayed shapes keep their answer.
    #[test]
    fn an_unstreamed_message_prints_once() {
        let mut renderer = AskRenderer::new();
        let mut out = String::new();
        renderer
            .render_to(&message("Hello world"), false, &mut out)
            .unwrap();
        assert_eq!(out, "Hello world\n");
    }

    /// A delta ending on a newline does not gain a blank line from the
    /// message that follows it.
    #[test]
    fn a_newline_terminated_stream_gains_no_blank_line() {
        let mut renderer = AskRenderer::new();
        let mut out = String::new();
        for event in [delta("Hello world\n"), message("Hello world")] {
            renderer.render_to(&event, false, &mut out).unwrap();
        }
        assert_eq!(out, "Hello world\n");
    }

    /// A new turn starts fresh: a message after a streamed previous turn
    /// prints when it arrived without deltas of its own.
    #[test]
    fn a_new_turn_resets_the_streaming_state() {
        let mut renderer = AskRenderer::new();
        let mut out = String::new();
        let turn = |input: &str| {
            event(SessionEventBody::TurnStarted(SessionTurnStartedProps {
                turn_id: TurnId::new(),
                input:   input.to_string(),
            }))
        };
        for event in [
            delta("First"),
            message("First"),
            turn("Second question"),
            message("Second answer"),
        ] {
            renderer.render_to(&event, false, &mut out).unwrap();
        }
        assert_eq!(out, "First\nSecond answer\n");
    }

    /// A tool call prints as a card: the call when it starts, its outcome
    /// when it completes.
    #[test]
    fn a_tool_call_renders_as_cards() {
        let mut renderer = AskRenderer::new();
        let mut out = String::new();
        let turn_id = TurnId::new();
        let started = event(SessionEventBody::ToolCallStarted(
            SessionToolCallStartedProps {
                turn_id,
                tool_name: "fabro_work_next".to_string(),
                tool_call_id: "call_1".to_string(),
                arguments: serde_json::Value::Null,
            },
        ));
        let completed = event(SessionEventBody::ToolCallCompleted(
            SessionToolCallCompletedProps {
                turn_id,
                tool_name: "fabro_work_next".to_string(),
                tool_call_id: "call_1".to_string(),
                output: serde_json::Value::Null,
                is_error: false,
                output_bytes_observed: None,
                output_bytes_retained: None,
                output_bytes_omitted: None,
            },
        ));
        let failed = event(SessionEventBody::ToolCallCompleted(
            SessionToolCallCompletedProps {
                turn_id,
                tool_name: "fabro_work_next".to_string(),
                tool_call_id: "call_2".to_string(),
                output: serde_json::Value::Null,
                is_error: true,
                output_bytes_observed: None,
                output_bytes_retained: None,
                output_bytes_omitted: None,
            },
        ));
        for event in [started, completed, failed] {
            renderer.render_to(&event, false, &mut out).unwrap();
        }
        assert_eq!(
            out,
            "[tool] fabro_work_next\n[tool] fabro_work_next ok\n[tool] fabro_work_next failed\n"
        );
    }

    /// A capability change prints as a card, on its own line after a
    /// delta stream caught mid-line.
    #[test]
    fn a_capability_change_renders_as_a_card() {
        let mut renderer = AskRenderer::new();
        let mut out = String::new();
        let changed = event(SessionEventBody::CapabilityChanged(
            SessionCapabilityChangedProps {
                capability: AskFabroCapability::FullAccess,
                actor:      None,
            },
        ));
        renderer
            .render_to(&delta("Flipping"), false, &mut out)
            .unwrap();
        renderer.render_to(&changed, false, &mut out).unwrap();
        assert_eq!(out, "Flipping\n[capability] full_access\n");
    }

    /// The announce line prints whole, ahead of the turn's stream.
    #[test]
    fn an_announce_prints_one_line() {
        let mut renderer = AskRenderer::new();
        let mut out = String::new();
        renderer.announce_to("[session] 01ABC", &mut out).unwrap();
        assert_eq!(out, "[session] 01ABC\n");
    }
}
