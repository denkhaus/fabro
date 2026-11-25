import type { PaginatedSessionList, SessionSummary } from "@qltysh/fabro-api-client";

import { apiData, sessionsApi } from "./api-client";

/** A session as the global index door serves it (ADR-0046, work#1452). */
export type AskSessionSummary = SessionSummary;

/** One page of the global index door, as the generated client types it. */
export type AskSessionIndexPage = PaginatedSessionList;

/** Loads one page of the global index through the generated client. */
function loadIndexPage(limit: number, offset: number): Promise<AskSessionIndexPage> {
  return apiData(() => sessionsApi.listSessions(limit, offset));
}

/**
 * The global session index: every Ask Fabro session across all runs, with
 * its run anchor, last activity and capability state. Rides the generated
 * client's `listSessions` operation (`GET /api/v1/sessions`; the generated
 * client is machine-owned — never hand-mirrored).
 */
export async function listAllAskSessions({
  limit = 100,
  offset = 0,
  loadPage = loadIndexPage,
}: {
  limit?: number;
  offset?: number;
  loadPage?: (limit: number, offset: number) => Promise<AskSessionIndexPage>;
} = {}): Promise<AskSessionSummary[]> {
  const page = await loadPage(limit, offset);
  return page.data;
}

/**
 * Explicitly deletes one session (work#1535) through the generated
 * client's `deleteSession` operation (`DELETE /api/v1/sessions/{id}`):
 * the session leaves the index and its transcript is no longer
 * reachable. Unlike the conversation reset door, deletion is final.
 */
export async function deleteAskSession(sessionId: string): Promise<void> {
  await apiData(() => sessionsApi.deleteSession(sessionId));
}

/**
 * The /ask page's last-active session memory (work#1686): the browser
 * remembers which conversation the operator had open, so returning to
 * the chat page restores it instead of the empty state. Writes happen
 * in event handlers (picker selection, a fresh conversation creating
 * its session); the restore itself derives during render against the
 * session index — the server-side index stays the truth, the memory
 * only names the candidate.
 */
const LAST_SESSION_KEY = "ask.lastSessionId";

/** The storage seam (test injects a fake; production rides the global). */
type SessionMemoryStorage = Pick<Storage, "getItem" | "setItem">;

export function rememberAskSessionId(
  sessionId: string,
  storage: SessionMemoryStorage | undefined = globalThis.localStorage,
): void {
  try {
    storage.setItem(LAST_SESSION_KEY, sessionId);
  } catch {
    // Storage unavailable (private mode, quota): the restore degrades
    // to the most recently updated session — never worth surfacing.
  }
}

export function recallAskSessionId(
  storage: SessionMemoryStorage | undefined = globalThis.localStorage,
): string | null {
  try {
    return storage.getItem(LAST_SESSION_KEY);
  } catch {
    return null;
  }
}

/** The selection the /ask page should show: the remembered conversation
 * when the index still carries it, otherwise the most recently updated
 * session (the index serves `order=updated_desc` by default, so the
 * first row is the newest activity) — or nothing on an empty index. */
export function restoreAskSelection(
  rememberedId: string | null,
  sessions: AskSessionSummary[],
): AskSessionSummary | null {
  const remembered =
    rememberedId != null
      ? sessions.find((session) => session.id === rememberedId)
      : undefined;
  return remembered ?? sessions[0] ?? null;
}
