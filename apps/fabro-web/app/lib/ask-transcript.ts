import type { ThreadMessageLike } from "@assistant-ui/react";

import { applyTurnEvent, type TurnAccumulator } from "./ask-fabro-runtime";
import { sessionsApi } from "./api-client";
import type { SessionStreamEvent } from "./session-stream";

const TRANSCRIPT_PAGE_LIMIT = 1000;
const TRANSCRIPT_MAX_EVENTS = 10_000;

/**
 * Replay a session's recorded events into assistant-ui thread messages
 * (work#1452): the model's context resumes server-side from pebble's
 * session record, and THIS replay restores the visible transcript beside
 * it. The fold reuses the live-turn accumulator (`applyTurnEvent`), so a
 * resumed transcript renders exactly like a live one.
 */
export function transcriptFromEvents(
  events: readonly SessionStreamEvent[],
): ThreadMessageLike[] {
  const messages: ThreadMessageLike[] = [];
  let acc: TurnAccumulator = {
    activeTextIndex: null,
    parts: [],
    toolCallIndex: new Map(),
  };

  const flushAssistant = () => {
    if (acc.parts.length > 0) {
      messages.push({ role: "assistant", content: acc.parts.slice() });
    }
    acc = { activeTextIndex: null, parts: [], toolCallIndex: new Map() };
  };

  for (const event of events) {
    const name = event.event;
    // The delta frames are streaming fragments; the assistant_message event
    // carries the turn's full text, so replaying deltas would duplicate it.
    if (name === "run.session.assistant_delta") continue;
    if (name === "run.session.user_message") {
      flushAssistant();
      const text = propsText(event, "text");
      if (text) messages.push({ role: "user", content: [{ type: "text", text }] });
      continue;
    }
    if (
      name === "run.session.turn.succeeded" ||
      name === "run.session.turn.failed" ||
      name === "run.session.turn.interrupted"
    ) {
      flushAssistant();
      continue;
    }
    applyTurnEvent(acc, event);
  }
  flushAssistant();
  return messages;
}

function propsText(event: SessionStreamEvent, key: string): string | null {
  if (typeof event.properties !== "object" || event.properties === null) return null;
  const value = (event.properties as Record<string, unknown>)[key];
  return typeof value === "string" ? value : null;
}

/**
 * Fetch a session's full event log page by page and fold it into thread
 * messages. Delta frames are skipped by the fold (their text lands in the
 * assistant_message events), so the transcript is complete without them.
 */
export async function loadSessionTranscript(
  sessionId: string,
): Promise<ThreadMessageLike[]> {
  const events: SessionStreamEvent[] = [];
  let sinceSeq = 1;
  while (events.length < TRANSCRIPT_MAX_EVENTS) {
    const page = (await sessionsApi.listSessionEvents(sessionId, sinceSeq, TRANSCRIPT_PAGE_LIMIT)).data;
    events.push(...page.data);
    if (!page.data.length || !page.meta?.has_more) break;
    sinceSeq = page.data[page.data.length - 1]!.seq + 1;
  }
  return transcriptFromEvents(events);
}
