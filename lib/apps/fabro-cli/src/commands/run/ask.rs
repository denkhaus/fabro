use std::io::Write;

use anyhow::{Result, bail};
use fabro_api::types::CreateRunSessionRequest;
use fabro_store::EventEnvelope;

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
    let stdout = std::io::stdout();
    let mut renderer = TextStreamRenderer::new(stdout.lock());
    while let Some(event) = stream.next_event().await? {
        render_event(&event, ctx.json_output(), &mut renderer)?;
        match event.event.event_name() {
            "run.session.turn.succeeded" | "run.session.turn.interrupted" => {
                saw_terminal = true;
            }
            "run.session.turn.failed" => {
                saw_terminal = true;
                terminal_error = Some(
                    event
                        .event
                        .properties()?
                        .get("error")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("session turn failed")
                        .to_string(),
                );
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

/// Renders streamed session events for the `ask` command.
///
/// In text mode, streamed assistant deltas own the answer output: once any
/// delta has been rendered, the final `assistant_message` only terminates the
/// line instead of printing the full text again.
struct TextStreamRenderer<W: Write> {
    out:       W,
    saw_delta: bool,
}

impl<W: Write> TextStreamRenderer<W> {
    fn new(out: W) -> Self {
        Self {
            out,
            saw_delta: false,
        }
    }

    fn render(&mut self, event: &EventEnvelope) -> Result<()> {
        match event.event.event_name() {
            "run.session.assistant_delta" => {
                let properties = event.event.properties()?;
                if let Some(delta) = properties.get("delta").and_then(serde_json::Value::as_str) {
                    write!(self.out, "{delta}")?;
                    self.saw_delta = true;
                }
            }
            "run.session.assistant_message" => {
                let properties = event.event.properties()?;
                let text = properties
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .filter(|text| !text.is_empty());
                if self.saw_delta {
                    // Deltas already rendered the content and left output
                    // mid-line; only terminate the line.
                    writeln!(self.out)?;
                } else if let Some(text) = text {
                    writeln!(self.out, "{text}")?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn render_event<W: Write>(
    event: &EventEnvelope,
    json_output: bool,
    renderer: &mut TextStreamRenderer<W>,
) -> Result<()> {
    if json_output {
        writeln!(renderer.out, "{}", serde_json::to_string(event)?)?;
        return Ok(());
    }
    renderer.render(event)
}

#[cfg(test)]
mod tests {
    use fabro_store::EventEnvelope;
    use serde_json::json;

    use super::TextStreamRenderer;

    fn envelope(event: &str, properties: serde_json::Value) -> EventEnvelope {
        let mut props = serde_json::Map::new();
        props.insert(
            "turn_id".to_string(),
            serde_json::json!("01ARZ3NDEKTSV4RRFFQ69G5FAV"),
        );
        if let serde_json::Value::Object(extra) = properties {
            props.extend(extra);
        }
        serde_json::from_value(json!({
            "seq": 1,
            "id": "evt_1",
            "ts": "2026-09-04T00:00:00Z",
            "run_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "event": event,
            "properties": props,
        }))
        .unwrap_or_else(|e| panic!("deserialize {event} failed: {e}"))
    }

    fn render_all(events: &[EventEnvelope]) -> String {
        let mut out = Vec::new();
        let mut renderer = TextStreamRenderer::new(&mut out);
        for event in events {
            super::render_event(event, false, &mut renderer).unwrap();
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn text_mode_delta_stream_prints_answer_exactly_once() {
        let events = [
            envelope(
                "run.session.assistant_delta",
                json!({"delta": "ECHO"}),
            ),
            envelope(
                "run.session.assistant_delta",
                json!({"delta": "9T"}),
            ),
            envelope(
                "run.session.assistant_message",
                json!({"text": "ECHO9T"}),
            ),
        ];
        assert_eq!(render_all(&events), "ECHO9T\n");
    }

    #[test]
    fn text_mode_without_deltas_prints_final_message_once() {
        let events = [envelope(
            "run.session.assistant_message",
            json!({"text": "ECHO9T"}),
        )];
        assert_eq!(render_all(&events), "ECHO9T\n");
    }

    #[test]
    fn json_mode_emits_every_event_once_as_json() {
        let events = [
            envelope(
                "run.session.assistant_delta",
                json!({"delta": "ECHO"}),
            ),
            envelope(
                "run.session.assistant_delta",
                json!({"delta": "9T"}),
            ),
            envelope(
                "run.session.assistant_message",
                json!({"text": "ECHO9T"}),
            ),
        ];
        let mut out = Vec::new();
        let mut renderer = TextStreamRenderer::new(&mut out);
        for event in &events {
            super::render_event(event, true, &mut renderer).unwrap();
        }
        let rendered = String::from_utf8(out).unwrap();
        let lines: Vec<&str> = rendered.lines().collect();
        assert_eq!(lines.len(), 3);
        let delta_count = lines
            .iter()
            .filter(|line| line.contains("run.session.assistant_delta"))
            .count();
        let message_count = lines
            .iter()
            .filter(|line| line.contains("run.session.assistant_message"))
            .count();
        assert_eq!(delta_count, 2);
        assert_eq!(message_count, 1);
        for line in lines {
            serde_json::from_str::<serde_json::Value>(line).unwrap();
        }
    }
}
