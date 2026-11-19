import { generatedApiConfiguration } from "./api-client";

/** A session as the global index door serves it (ADR-0046, work#1452). */
export interface AskSessionSummary {
  id: string;
  run_id: string;
  title: string | null;
  status: string;
  model?: string | null;
  capability?: string;
  created_at: string;
  updated_at: string;
}

interface IndexPage {
  data: AskSessionSummary[];
  meta: { has_more: boolean };
}

/**
 * The global session index: every Ask Fabro session across all runs, with
 * its run anchor, last activity and capability state. Calls the
 * `GET /api/v1/sessions` door by hand until the generated API client's
 * regen picks the endpoint up (the generated client is machine-owned —
 * never hand-mirrored).
 */
export async function listAllAskSessions({
  limit = 100,
  offset = 0,
  fetchImpl = fetch,
}: {
  limit?: number;
  offset?: number;
  fetchImpl?: typeof fetch;
} = {}): Promise<AskSessionSummary[]> {
  const params = new URLSearchParams({
    "page[limit]": String(limit),
    "page[offset]": String(offset),
  });
  const base = generatedApiConfiguration.basePath ?? "";
  const response = await fetchImpl(`${base}/api/v1/sessions?${params}`, {
    credentials: "include",
  });
  if (!response.ok) {
    throw new Error(`session index request failed: ${response.status}`);
  }
  const page = (await response.json()) as IndexPage;
  return page.data;
}
