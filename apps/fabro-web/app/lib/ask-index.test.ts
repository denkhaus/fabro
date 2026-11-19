import { describe, expect, test } from "bun:test";

import { listAllAskSessions } from "./ask-index";

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

describe("listAllAskSessions", () => {
  test("fetches the global index door with pagination params", async () => {
    const calls: string[] = [];
    const sessions = await listAllAskSessions({
      limit: 5,
      offset: 10,
      fetchImpl: (async (input: string | URL | Request) => {
        calls.push(String(input));
        return jsonResponse({
          data: [
            {
              id: "ses_1",
              run_id: "run_1",
              title: "Ask Fabro",
              status: "idle",
              capability: "full_access",
              created_at: "2026-11-19T10:00:00Z",
              updated_at: "2026-11-19T11:00:00Z",
            },
          ],
          meta: { has_more: false },
        });
      }) as typeof fetch,
    });

    expect(calls).toEqual(["/api/v1/sessions?page%5Blimit%5D=5&page%5Boffset%5D=10"]);
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
        fetchImpl: (async () => jsonResponse({}, 500)) as typeof fetch,
      });
      throw new Error("expected throw");
    } catch (error) {
      expect((error as Error).message).toContain("500");
    }
  });
});
