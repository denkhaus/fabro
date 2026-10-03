use anyhow::{Result, bail};
use fabro_api::types::CreateRunSessionRequest;
use fabro_types::{SessionEvent, SessionEventBody};

use crate::args::AskArgs;
use crate::command_context::CommandContext;

pub(crate) async fn run(args: AskArgs, base_ctx: &CommandContext) -> Result<()> {
    let ctx = base_ctx.with_target(&args.server)?;
    let client = ctx.server().await?;
    let run_id = client.resolve_run(&args.run).await?.id;
    let session = client
        .create_run_session(run_id, CreateRunSessionRequest {
            title:    Some(session_title(&args.prompt)),
            model:    args.model,
            provider: None,
        })
        .await?;
    let mut stream = client
        .submit_session_turn_stream(session.id, args.prompt)
        .await?;

    let mut terminal_error = None;
    let mut saw_terminal = false;
    let mut renderer = AskRenderer::new();
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
/// preceded (a non-streaming model, a replayed turn) prints once.
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
            _ => {}
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
        SessionAssistantDeltaProps, SessionAssistantMessageProps, SessionTurnStartedProps,
    };
    use fabro_types::{RunId, SessionId, TurnId};

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
}
