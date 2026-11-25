import { describe, expect, mock, test } from "bun:test";

// Partial-mock (the run-detail.test.ts pattern): bun's mock.module is
// process-wide, so the runless-door tests live in their own file. The
// REAL api-client module is captured FIRST and re-spread into the mock —
// only the sessionsApi object is replaced — so every other export
// (apiErrorFromFetchResponse, generatedApiConfiguration, …) stays real.
import * as realApiClient from "./api-client";

const doorCalls: { runless: number; runScoped: number } = {
  runless: 0,
  runScoped: 0,
};

mock.module("./api-client", () => ({
  ...realApiClient,
  sessionsApi: {
    ...realApiClient.sessionsApi,
    // The runless door: POST /sessions with no run anchor.
    createSession: async (_body: unknown) => {
      doorCalls.runless += 1;
      return { data: { id: "ses_runless" } };
    },
    // The run-scoped door: POST /runs/{run_id}/sessions.
    createRunSession: async (_runId: string, _body: unknown) => {
      doorCalls.runScoped += 1;
      return { data: { id: "ses_run_scoped" } };
    },
  },
}));

const { createAskFabroAdapter } = await import("./ask-fabro-runtime");

interface UserMessagesShape {
  role: string;
  content: ReadonlyArray<{ type: string; text: string }>;
}

function userMessages(text: string): Array<UserMessagesShape> {
  return [{ role: "user", content: [{ type: "text", text }] }];
}

function fakeRunArgs(signal: AbortSignal, messages: Array<UserMessagesShape>) {
  return {
    signal,
    messages,
    abstractions: {},
    runConfig: {},
    context: { tools: [] },
    unstable_getMessage: () => ({}) as never,
  } as unknown as Parameters<
    ReturnType<typeof createAskFabroAdapter>["run"]
  >[0];
}

async function drain(result: AsyncIterable<unknown>) {
  for await (const _ of result) {
    // drain
  }
}

describe("the runless session door (work#1678)", () => {
  test("an adapter without a run creates the session through the runless door", async () => {
    const adapter = createAskFabroAdapter({
      defaultModel: "claude-haiku-4-5",
      streamSessionTurnImpl: async (args) => {
        args.onEvent({
          type: "run.session.assistant_delta",
          seq: 1,
          session_id: "ses_runless",
          run_id: null,
          ts: "2026-11-25T00:00:00Z",
          properties: { delta: "Hello" },
        } as never);
        return { turnId: "turn_1" };
      },
    });

    const ctl = new AbortController();
    await drain(adapter.run(fakeRunArgs(ctl.signal, userMessages("Say hi"))));

    expect(doorCalls.runless).toBe(1);
    expect(doorCalls.runScoped).toBe(0);
  });

  test("an adapter with a run keeps using the run-scoped door", async () => {
    const adapter = createAskFabroAdapter({
      runId: "run_1",
      defaultModel: "claude-haiku-4-5",
      streamSessionTurnImpl: async () => ({ turnId: "turn_1" }),
    });

    const ctl = new AbortController();
    await drain(adapter.run(fakeRunArgs(ctl.signal, userMessages("hi"))));

    expect(doorCalls.runScoped).toBe(1);
  });
});
