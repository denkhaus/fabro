import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import { createElement } from "react";
import TestRenderer, { act } from "react-test-renderer";
import { SWRConfig } from "swr";

import { setupReactTestEnv, textContent } from "../lib/test-utils";

// Deferred sessions API: the tests control exactly when the index and the
// event log resolve, so the page's mount-time behavior is observed
// deterministically (work#1507 — the page must read the sessions index on
// mount and rehydrate a picked conversation's transcript).
let indexResolvers: Array<(page: unknown) => void> = [];
let eventResolvers: Array<(page: unknown) => void> = [];
let listSessionEventsCalls = 0;
let deletedSessionIds: string[] = [];

function makeIndexPage(sessionId = "ses_1", title = "Supervisor chat") {
  return {
    data: [
      {
        id: sessionId,
        run_id: "run_1",
        title,
        status: "idle",
        model: null,
        capability: "full_access",
        created_at: "2026-11-20T12:00:00Z",
        updated_at: "2026-11-20T12:05:00Z",
      },
    ],
    meta: { has_more: false },
  };
}

function makeEventsPage() {
  return {
    data: [
      {
        seq: 1,
        session_id: "ses_1",
        run_id: "run_1",
        ts: "2026-11-20T12:01:00Z",
        event: "run.session.user_message",
        properties: { text: "What is the status?" },
      },
      {
        seq: 2,
        session_id: "ses_1",
        run_id: "run_1",
        ts: "2026-11-20T12:02:00Z",
        event: "run.session.assistant_message",
        properties: { text: "All stages are green." },
      },
    ],
    meta: { has_more: false },
  };
}

// Partial-mock: bun's mock.module is process-wide (restore does not undo
// it) — spread the REAL module so every export stays available to later
// files in the same run; only the sessions API object is replaced. The real
// ask-index seam (`listAllAskSessions` → `sessionsApi.listSessions`) and the
// real SWR hooks stay in the loop.
const realApiClient = await import("../lib/api-client");
mock.module("../lib/api-client", () => ({
  ...realApiClient,
  sessionsApi: {
    ...realApiClient.sessionsApi,
    listSessions: mock(() =>
      new Promise((resolve) => {
        indexResolvers.push(resolve);
      }).then((page) => ({ data: page })),
    ),
    listSessionEvents: mock(() => {
      listSessionEventsCalls += 1;
      return new Promise((resolve) => {
        eventResolvers.push(resolve);
      }).then((page) => ({ data: page }));
    }),
    deleteSession: mock((id: string) => {
      deletedSessionIds.push(id);
      return Promise.resolve({});
    }),
  },
}));

// The real `Thread` pulls in radix-ui tooltips that need `window`/`document`,
// which react-test-renderer does not provide. This probe renders the SAME
// runtime state the real Thread renders — the restored messages, and the
// welcome screen on an empty thread — through the real `useAuiState` hook,
// so the runtime's construction behavior (the mount-time `initialMessages`
// import) stays real under test.
const realAssistantUi = await import("@assistant-ui/react");
const realReactUi = await import("@assistant-ui/react-ui");
type ProbeMessage = {
  role: string;
  content: ReadonlyArray<{ type: string; text?: string }>;
};
mock.module("@assistant-ui/react-ui", () => ({
  ...realReactUi,
  Thread: (props: {
    components?: { ThreadWelcome?: React.ComponentType | null };
  }) => {
    const messages = realAssistantUi.useAuiState(
      (s: { thread: { messages: ProbeMessage[] } }) => s.thread.messages,
    );
    const welcome = messages.length === 0
      ? props.components?.ThreadWelcome
      : null;
    return createElement(
      "div",
      { "data-probe": "thread" },
      welcome ? createElement(welcome) : null,
      messages.map((message, i) =>
        createElement(
          "div",
          { key: i, "data-role": message.role },
          message.content
            .filter((part) => part.type === "text")
            .map((part, j) => createElement("span", { key: j }, part.text)),
        ),
      ),
    );
  },
}));

const { default: Ask } = await import("./ask");
mock.restore();

// Trees mount with deduping disabled so every mount's revalidation fires
// deterministically (the default 2s dedupe window would otherwise swallow
// back-to-back mounts across tests); the cache provider stays the default
// shared one, which is exactly the SPA-revisit situation under test.
function mountAsk(): TestRenderer.ReactTestRenderer {
  let renderer: TestRenderer.ReactTestRenderer = null as never;
  act(() => {
    renderer = TestRenderer.create(
      createElement(
        SWRConfig,
        { value: { dedupingInterval: 0 } },
        createElement(Ask),
      ),
    );
  });
  return renderer;
}

function flush(times = 6) {
  let chain = Promise.resolve();
  for (let i = 0; i < times; i++) {
    chain = chain.then(() => Promise.resolve());
  }
  return chain;
}

async function resolveAll(
  resolvers: Array<(page: unknown) => void>,
  page: unknown,
) {
  const pending = resolvers.splice(0);
  act(() => {
    for (const resolve of pending) resolve(page);
  });
  await act(async () => {
    await flush();
  });
}

function textOf(tree: TestRenderer.ReactTestRenderer): string {
  return textContent(tree.root);
}

function buttonsOf(tree: TestRenderer.ReactTestRenderer) {
  return tree.root.findAll((node) => node.type === "button");
}

describe("Ask page rehydration (work#1507)", () => {
  let teardown: () => void = () => {};
  let tree: TestRenderer.ReactTestRenderer | null = null;

  beforeEach(() => {
    teardown = setupReactTestEnv();
    indexResolvers = [];
    eventResolvers = [];
    listSessionEventsCalls = 0;
    deletedSessionIds = [];
  });

  afterEach(() => {
    act(() => {
      tree?.unmount();
    });
    tree = null;
    teardown();
  });

  test("the page reads the sessions index on mount and lists the conversation", async () => {
    tree = mountAsk();

    expect(indexResolvers.length).toBeGreaterThan(0);

    await resolveAll(indexResolvers, makeIndexPage());

    const text = textOf(tree!);
    expect(text).toContain("Supervisor chat");
  });

  test("picking a conversation restores its transcript instead of the empty state", async () => {
    tree = mountAsk();
    await resolveAll(indexResolvers, makeIndexPage());

    const conversationButton = buttonsOf(tree!).find((button) =>
      textContent(button).includes("Supervisor chat"),
    );
    expect(conversationButton).toBeDefined();

    act(() => {
      conversationButton!.props.onClick();
    });
    await act(async () => {
      await flush();
    });

    expect(listSessionEventsCalls).toBeGreaterThan(0);
    await resolveAll(eventResolvers, makeEventsPage());

    const text = textOf(tree!);
    expect(text).toContain("What is the status?");
    expect(text).toContain("All stages are green.");
    expect(text).not.toContain("How can I help?");
  });

  test("revisiting a conversation refetches the transcript and surfaces a turn that completed while away", async () => {
    // First visit — the conversation is mid-turn: the question is recorded,
    // no answer yet (the turn is still running server-side).
    tree = mountAsk();
    await resolveAll(indexResolvers, makeIndexPage("ses_mid", "Deploy check"));

    const firstButton = buttonsOf(tree!).find((button) =>
      textContent(button).includes("Deploy check"),
    );
    expect(firstButton).toBeDefined();
    act(() => {
      firstButton!.props.onClick();
    });
    await act(async () => {
      await flush();
    });
    const firstVisitFetches = listSessionEventsCalls;
    expect(firstVisitFetches).toBeGreaterThan(0);
    await resolveAll(eventResolvers, {
      data: [
        {
          seq: 1,
          session_id: "ses_mid",
          run_id: "run_1",
          ts: "2026-11-20T12:01:00Z",
          event: "run.session.user_message",
          properties: { text: "Where is the deploy log?" },
        },
      ],
      meta: { has_more: false },
    });
    expect(textOf(tree!)).toContain("Where is the deploy log?");
    expect(textOf(tree!)).not.toContain("It is in tmp/deploy.log.");

    // Navigate away and back (a fresh tree, same SPA session: SWR's
    // in-memory cache carries over — the server completed the turn in
    // between, so the events door now carries the answer).
    act(() => {
      tree?.unmount();
    });
    tree = mountAsk();
    await resolveAll(indexResolvers, makeIndexPage("ses_mid", "Deploy check"));

    const revisitButton = buttonsOf(tree!).find((button) =>
      textContent(button).includes("Deploy check"),
    );
    expect(revisitButton).toBeDefined();
    act(() => {
      revisitButton!.props.onClick();
    });
    await act(async () => {
      await flush();
    });

    // The revisit must re-read the events door (not replay a stale cache).
    expect(listSessionEventsCalls).toBeGreaterThan(firstVisitFetches);
    await resolveAll(eventResolvers, {
      data: [
        {
          seq: 1,
          session_id: "ses_mid",
          run_id: "run_1",
          ts: "2026-11-20T12:01:00Z",
          event: "run.session.user_message",
          properties: { text: "Where is the deploy log?" },
        },
        {
          seq: 2,
          session_id: "ses_mid",
          run_id: "run_1",
          ts: "2026-11-20T12:03:00Z",
          event: "run.session.assistant_message",
          properties: { text: "It is in tmp/deploy.log." },
        },
      ],
      meta: { has_more: false },
    });

    const text = textOf(tree!);
    expect(text).toContain("Where is the deploy log?");
    expect(text).toContain("It is in tmp/deploy.log.");
  });
});

describe("Ask page session delete (work#1535)", () => {
  let teardown: () => void = () => {};
  let tree: TestRenderer.ReactTestRenderer | null = null;

  beforeEach(() => {
    teardown = setupReactTestEnv();
    indexResolvers = [];
    eventResolvers = [];
    listSessionEventsCalls = 0;
    deletedSessionIds = [];
  });

  afterEach(() => {
    act(() => {
      tree?.unmount();
    });
    tree = null;
    teardown();
  });

  test("a session row's delete affordance calls the door and removes the row", async () => {
    tree = mountAsk();
    await resolveAll(indexResolvers, {
      data: [
        {
          id: "ses_1",
          run_id: "run_1",
          title: "Supervisor chat",
          status: "idle",
          model: null,
          capability: "full_access",
          created_at: "2026-11-20T12:00:00Z",
          updated_at: "2026-11-20T12:05:00Z",
        },
        {
          id: "ses_2",
          run_id: "run_2",
          title: "Deploy check",
          status: "idle",
          model: null,
          capability: "read_only",
          created_at: "2026-11-20T13:00:00Z",
          updated_at: "2026-11-20T13:05:00Z",
        },
      ],
      meta: { has_more: false },
    });
    expect(textOf(tree!)).toContain("Supervisor chat");
    expect(textOf(tree!)).toContain("Deploy check");

    // The affordance is confirm-less: one click deletes.
    const deleteButton = buttonsOf(tree!).find(
      (button) => button.props["aria-label"] === "Delete conversation Supervisor chat",
    );
    expect(deleteButton).toBeDefined();

    act(() => {
      deleteButton!.props.onClick({
        stopPropagation: () => {},
      });
    });
    await act(async () => {
      await flush();
    });

    // The door was called for exactly the picked session.
    expect(deletedSessionIds).toEqual(["ses_1"]);

    // The refetched index no longer carries the deleted row; the sibling
    // stays listed.
    await resolveAll(indexResolvers, {
      data: [
        {
          id: "ses_2",
          run_id: "run_2",
          title: "Deploy check",
          status: "idle",
          model: null,
          capability: "read_only",
          created_at: "2026-11-20T13:00:00Z",
          updated_at: "2026-11-20T13:05:00Z",
        },
      ],
      meta: { has_more: false },
    });
    expect(textOf(tree!)).not.toContain("Supervisor chat");
    expect(textOf(tree!)).toContain("Deploy check");
  });
});
