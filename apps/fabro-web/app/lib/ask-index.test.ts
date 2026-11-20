import { describe, expect, test } from "bun:test";

import type { PaginatedSessionList, SessionSummary } from "@qltysh/fabro-api-client";

import { listAllAskSessions } from "./ask-index";

function sessionSummary(overrides: Partial<SessionSummary>): SessionSummary {
  return {
    id: "ses_1",
    run_id: "run_1",
    title: "Ask Fabro",
    status: "idle",
    active_turn: null,
    created_at: "2026-11-19T10:00:00Z",
    updated_at: "2026-11-19T11:00:00Z",
    ...overrides,
  };
}

function indexPage(data: SessionSummary[]): PaginatedSessionList {
  return { data, meta: { has_more: false } };
}

describe("listAllAskSessions", () => {
  test("rides the generated index door with pagination params and unwraps the page", async () => {
    const calls: Array<[number, number]> = [];
    const sessions = await listAllAskSessions({
      limit: 5,
      offset: 10,
      loadPage: async (limit, offset) => {
        calls.push([limit, offset]);
        return indexPage([
          sessionSummary({ capability: "full_access" }),
        ]);
      },
    });

    expect(calls).toEqual([[5, 10]]);
    expect(sessions).toHaveLength(1);
    expect(sessions[0]).toMatchObject({
      id: "ses_1",
      run_id: "run_1",
      capability: "full_access",
    });
  });

  test("a failed request throws", async () => {
    try {
      await listAllAskSessions({
        loadPage: async () => {
          throw new Error("session index request failed: 500");
        },
      });
      throw new Error("expected throw");
    } catch (error) {
      expect((error as Error).message).toContain("500");
    }
  });
});
