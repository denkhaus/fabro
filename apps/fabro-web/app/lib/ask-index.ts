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
