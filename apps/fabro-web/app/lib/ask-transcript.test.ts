import { describe, expect, test } from "bun:test";

import { transcriptFromEvents } from "./ask-transcript";
import type { SessionStreamEvent } from "./session-stream";

function event(
  seq: number,
  name: string,
  properties: Record<string, unknown>,
): SessionStreamEvent {
  return {
    seq,
    session_id: "ses_1",
    run_id: "run_1",
    ts: "2026-11-19T12:00:00Z",
    event: name,
    properties,
  } as unknown as SessionStreamEvent;
}

describe("transcriptFromEvents", () => {
  test("folds recorded events into user and assistant messages", () => {
    const messages = transcriptFromEvents([
      event(1, "run.session.created", { title: "Ask Fabro" }),
      event(2, "run.session.user_message", { text: "What happened?" }),
      event(3, "run.session.assistant_delta", { delta: "ignored" }),
      event(4, "run.session.assistant_message", { text: "All good." }),
      event(5, "run.session.turn.succeeded", {}),
    ]);

    expect(messages).toEqual([
      { role: "user", content: [{ type: "text", text: "What happened?" }] },
      { role: "assistant", content: [{ type: "text", text: "All good." }] },
    ]);
  });

  test("keeps tool calls as assistant content parts across turns", () => {
    const messages = transcriptFromEvents([
      event(1, "run.session.user_message", { text: "check" }),
      event(
        2,
        "run.session.tool_call.started",
        { tool_call_id: "tc_1", tool_name: "fabro_run_events", arguments: {} },
      ),
      event(
        3,
        "run.session.tool_call.completed",
        { tool_call_id: "tc_1", tool_name: "fabro_run_events", output: {}, is_error: false },
      ),
      event(4, "run.session.assistant_message", { text: "Done." }),
      event(5, "run.session.turn.succeeded", {}),
      event(6, "run.session.user_message", { text: "again" }),
      event(7, "run.session.turn.succeeded", {}),
    ]);

    expect(messages).toHaveLength(3);
    expect(messages[0]).toMatchObject({ role: "user" });
    expect(messages[1]?.role).toBe("assistant");
    const parts = (messages[1] as { content: Array<{ type: string }> }).content;
    expect(parts.some((part) => part.type === "tool-call")).toBe(true);
    expect(parts.some((part) => part.type === "text")).toBe(true);
    expect(messages[2]).toMatchObject({ role: "user" });
  });

  test("a failed turn folds to a visible failure note (work#1681)", () => {
    const messages = transcriptFromEvents([
      event(1, "run.session.user_message", { text: "Say hello." }),
      event(2, "run.session.turn.failed", {
        turn_id: "t_1",
        error: "run sandbox was not created",
        code: "sandbox_unavailable",
        retryable: false,
      }),
    ]);

    expect(messages).toHaveLength(2);
    expect(messages[0]).toMatchObject({
      role: "user",
      content: [{ type: "text", text: "Say hello." }],
    });
    expect(messages[1]).toMatchObject({
      role: "assistant",
      content: [{ type: "text", text: "Turn failed: run sandbox was not created" }],
    });
  });

  test("an interrupted turn folds to a visible note (work#1681)", () => {
    const messages = transcriptFromEvents([
      event(1, "run.session.user_message", { text: "Stop." }),
      event(2, "run.session.turn.interrupted", { turn_id: "t_1" }),
    ]);

    expect(messages).toHaveLength(2);
    expect(messages[1]).toMatchObject({
      role: "assistant",
      content: [{ type: "text", text: "Turn interrupted." }],
    });
  });

  test("an empty event log folds to an empty transcript", () => {
    expect(transcriptFromEvents([])).toEqual([]);
  });
});
