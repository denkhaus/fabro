import { afterAll, beforeAll, describe, expect, test } from "bun:test";
import TestRenderer, { act } from "react-test-renderer";
import { MemoryRouter } from "react-router";

import {
  AgentSessionActivity,
  CompactionReason,
  ContextWindowCategory,
  ContextWindowCountMethod,
  ContextWindowStaleness,
  FailoverContinuation,
  FailoverStop,
  SkillActivationSource,
  TodoListKind,
  TodoStatus,
  ToolCategory,
} from "@qltysh/fabro-api-client";
import type {
  AgentErrorData,
  AgentSessionProjection,
  McpToolSummary,
  StageContextWindow,
  StageProjection,
  Usage,
} from "@qltysh/fabro-api-client";

import { StageInsightsSidebar } from "./stage-insights-sidebar";

const NO_USAGE: Usage = {
  tokens: { input: 0, output: 0, reasoning: 0, cache_read: 0, cache_write: 0 },
};

function makeStage(overrides: Partial<StageProjection> = {}): StageProjection {
  return {
    first_event_seq: 1,
    state:           "running",
    usage:           NO_USAGE,
    ...overrides,
  };
}

/** The coding agent's fold of a stage that has seen nothing yet. */
function makeAgent(overrides: Partial<AgentSessionProjection> = {}): AgentSessionProjection {
  return {
    root_session_id: "ses_root",
    route:           { provider: "anthropic", model: "claude-opus-4-7" },
    activity:        AgentSessionActivity.RUNNING,
    usage:           NO_USAGE,
    messages:        0,
    descendants:     {},
    context_window:  null,
    tools:           {},
    mcp_servers:     {},
    skills:          { available: [], activated: [] },
    subagent_counts: { spawned: 0, turns_started: 0, completed: 0, failed: 0, closed: 0 },
    todos:           {},
    subagents:       [],
    compactions:     [],
    failovers:       [],
    files_touched:   [],
    last_file_touched: null,
    prompts:         1,
    prompt:          {
      completed:         false,
      usage:             NO_USAGE,
      messages:          0,
      context_window:    null,
      tool_calls:        0,
      descendants:       {},
      subagents:         { spawned: 0, turns_started: 0, completed: 0, failed: 0, closed: 0 },
      compactions:       [],
      files_touched:     [],
      last_file_touched: null,
    },
    ...overrides,
  };
}

function mcpTools(count: number): McpToolSummary[] {
  return Array.from({ length: count }, (_, i) => ({
    name:          `mcp__server__tool_${i}`,
    original_name: `tool_${i}`,
  }));
}

function agentError(message: string): AgentErrorData {
  return { kind: "llm", message };
}

function makeContextWindow(overrides: Partial<StageContextWindow> = {}): StageContextWindow {
  return {
    stage_id:              "implement@1",
    available:             true,
    unavailable_reason:    null,
    provider:              "anthropic",
    model:                 "claude-opus-4-7",
    context_window_tokens: 200_000,
    input_tokens:          62_000,
    usage_percent:         31,
    count_method:          ContextWindowCountMethod.PROVIDER_API_SCALED_BREAKDOWN,
    staleness:             ContextWindowStaleness.LIVE,
    generated_at:          new Date().toISOString(),
    event_seq:             42,
    breakdown:             [
      { category: ContextWindowCategory.SYSTEM_PROMPT, tokens: 8_000, usage_percent: 4 },
      { category: ContextWindowCategory.TOOLS, tokens: 12_000, usage_percent: 6 },
      { category: ContextWindowCategory.CONVERSATION, tokens: 42_000, usage_percent: 21 },
    ],
    warnings: [],
    ...overrides,
  };
}

// bun:test runs in a node-like env without a DOM, so shim `window.localStorage`
// once — the sidebar feature-detects `typeof window` to decide whether to
// persist collapse state. Seeding the shim lets us open default-collapsed
// sections in the assertions below. Other test files (e.g.
// services-panel.test.tsx) install their own window and rely on
// `delete globalThis.window` cleanup, so this descriptor stays configurable.
let restoreWindow: (() => void) | null = null;
beforeAll(() => {
  const store = new Map<string, string>();
  for (const key of ["todos", "context", "files", "subagents", "tools", "skills", "mcps"]) {
    store.set(`fabro:stage-insights-section:${key}`, "1");
  }
  const stub = {
    localStorage: {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => {
        store.set(key, value);
      },
    },
  };
  const had = "window" in globalThis;
  const prev = (globalThis as { window?: unknown }).window;
  Object.defineProperty(globalThis, "window", { value: stub, writable: true, configurable: true });
  restoreWindow = () => {
    if (had) {
      Object.defineProperty(globalThis, "window", { value: prev, writable: true, configurable: true });
    } else {
      delete (globalThis as { window?: unknown }).window;
    }
  };
});

afterAll(() => {
  restoreWindow?.();
  restoreWindow = null;
});

function render(stage: StageProjection | undefined, contextWindow: StageContextWindow | null): string {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  let renderer!: TestRenderer.ReactTestRenderer;
  act(() => {
    renderer = TestRenderer.create(
      <MemoryRouter>
        <StageInsightsSidebar stage={stage} contextWindow={contextWindow} />
      </MemoryRouter>,
    );
  });
  return JSON.stringify(renderer.toJSON());
}

describe("StageInsightsSidebar", () => {
  test("renders the root agent's todo list and counts the subagent lists apart", () => {
    const stage = makeStage({
      agent: makeAgent({
        todos: {
          "anthropic_tasks:ses_root": {
            kind:    TodoListKind.ANTHROPIC_TASKS,
            list_id: "anthropic_tasks:ses_root",
            items:   [
              { id: "1", status: TodoStatus.COMPLETED, order: 0, subject: "Plan refactor" },
              { id: "2", status: TodoStatus.COMPLETED, order: 1, subject: "Add tests" },
              { id: "3", status: TodoStatus.IN_PROGRESS, order: 2, subject: "Land migration" },
              { id: "4", status: TodoStatus.PENDING, order: 3, subject: "Review with Kieran" },
            ],
          },
          "openai_plan:ses_child": {
            kind:    TodoListKind.OPENAI_PLAN,
            list_id: "openai_plan:ses_child",
            items:   [{ id: "c1", status: TodoStatus.PENDING, order: 0, subject: "Child plan step" }],
          },
        },
      }),
    });
    const dom = render(stage, null);
    expect(dom).toContain("2/4");
    expect(dom).toContain("Plan refactor");
    expect(dom).toContain("Land migration");
    expect(dom).not.toContain("Child plan step");
    expect(dom).toContain("+1 subagent list");
  });

  test("renders context window percent and breakdown labels", () => {
    const dom = render(makeStage(), makeContextWindow());
    expect(dom).toContain("31%");
    expect(dom).toContain("System prompt");
    expect(dom).toContain("Conversation");
  });

  test("hides breakdown labels in unavailable state but still renders bar", () => {
    const cw = makeContextWindow({
      available:          false,
      usage_percent:      null,
      input_tokens:       null,
      staleness:          ContextWindowStaleness.UNAVAILABLE,
      unavailable_reason: null,
    });
    const dom = render(makeStage(), cw);
    expect(dom).toContain("--");
    expect(dom).not.toContain("31%");
  });

  test("renders the compactions row under the context window", () => {
    const stage = makeStage({
      agent: makeAgent({
        compactions: [
          {
            reason:                 CompactionReason.THRESHOLD,
            original_turn_count:    20,
            preserved_turn_count:   6,
            summary_token_estimate: 500,
            tracked_file_count:     3,
          },
        ],
      }),
    });
    const dom = render(stage, makeContextWindow());
    expect(dom).toContain("1 compaction · last kept 6 of 20 turns");
  });

  test("renders projected agent tool names and invoked state", () => {
    const dom = render(
      makeStage({
        agent_tools: [
          {
            name:        "apply_patch",
            description: "Apply a unified diff patch",
            source:      { kind: "native" },
            category:    ToolCategory.WRITE,
            invoked:     true,
          },
          {
            name:        "grep",
            description: "Search file contents",
            source:      { kind: "native" },
            category:    ToolCategory.READ,
            invoked:     false,
          },
        ],
      }),
      null,
    );

    expect(dom).toContain("1/2");
    expect(dom).toContain("apply_patch");
    expect(dom).toContain("grep");
    // Tool description still appears as the row `title` tooltip.
    expect(dom).toContain("Apply a unified diff patch");
    expect(dom).toContain("Search file contents");
    expect(dom).toContain("Used");
  });

  test("derives mcp server status from the fold and marks invoked servers as 'used'", () => {
    const dom = render(
      makeStage({
        agent: makeAgent({
          mcp_servers: {
            context7:   { tools: mcpTools(12), error: null, invoked: true },
            filesystem: { tools: mcpTools(3), error: null, invoked: false },
            atlassian:  { tools: [], error: "auth failed", invoked: false },
          },
        }),
      }),
      null,
    );
    // Used count badge in the section header (1 of 3 invoked).
    expect(dom).toContain("1/3");
    expect(dom).toContain("context7");
    expect(dom).toContain("used");
    expect(dom).toContain("filesystem");
    expect(dom).toContain("3 tools");
    expect(dom).toContain("atlassian");
    expect(dom).toContain("Failed");
    expect(dom).toContain("auth failed");
  });

  test("renders a disconnected mcp server as disconnected, still counted as used", () => {
    const dom = render(
      makeStage({
        agent: makeAgent({
          mcp_servers: {
            github: { tools: mcpTools(4), error: null, invoked: true, disconnected: "transport closed" },
          },
        }),
      }),
      null,
    );
    expect(dom).toContain("1/1");
    expect(dom).toContain("github");
    expect(dom).toContain("Disconnected");
    expect(dom).not.toContain("Failed");
  });

  test("shows skill activated/available ratio with source label", () => {
    const dom = render(
      makeStage({
        agent: makeAgent({
          skills: {
            activated: [
              { name: "frontend-design", source: SkillActivationSource.SLASH },
              { name: "debug", source: SkillActivationSource.TOOL },
            ],
            available: [
              { name: "frontend-design", description: "" },
              { name: "debug", description: "" },
              { name: "tdd", description: "" },
              { name: "ce-review", description: "" },
            ],
          },
        }),
      }),
      null,
    );
    expect(dom).toContain("2/4");
    expect(dom).toContain("frontend-design");
    expect(dom).toContain("slash");
    expect(dom).toContain("+2 more available");
  });

  test("lists the files the session tree wrote and marks the last one", () => {
    const dom = render(
      makeStage({
        agent: makeAgent({
          files_touched:     ["/workspace/src/lib.rs", "/workspace/src/main.rs"],
          last_file_touched: "/workspace/src/main.rs",
        }),
      }),
      null,
    );
    expect(dom).toContain("src/lib.rs");
    expect(dom).toContain("src/main.rs");
    expect(dom).toContain("last");
    // Full paths stay in the tooltip.
    expect(dom).toContain("/workspace/src/lib.rs");
  });

  test("renders subagents with their status", () => {
    const dom = render(
      makeStage({
        agent: makeAgent({
          subagents: [
            { agent_id: "sub-1", depth: 1, task: "Review the module", status: { status: "completed", success: true, turns_used: 3 } },
            { agent_id: "sub-2", depth: 1, task: "Check the tests", status: { status: "failed", error: agentError("boom") } },
            { agent_id: "sub-3", depth: 1, task: "Still looking", status: { status: "running" } },
          ],
        }),
      }),
      null,
    );
    expect(dom).toContain("2/3");
    expect(dom).toContain("Review the module");
    expect(dom).toContain("3 turns");
    expect(dom).toContain("Check the tests");
    expect(dom).toContain("Failed");
    expect(dom).toContain("boom");
    expect(dom).toContain("Still looking");
    expect(dom).toContain("running");
  });

  test("shows a failover badge with the route the session moved to and why it stopped", () => {
    const dom = render(
      makeStage({
        agent: makeAgent({
          route:     { provider: "openai", model: "gpt-5.4" },
          failovers: [
            {
              from:         "anthropic/claude-opus-4-7",
              to:           "openai/gpt-5.4",
              attempt:      1,
              error:        agentError("rate limited"),
              usage:        NO_USAGE,
              inference_ms: 120,
              tool_ms:      30,
              continuation: FailoverContinuation.CONTINUE_TURN,
            },
          ],
          failover_stopped: {
            route:   "openai/gpt-5.4",
            attempt: 1,
            reason:  FailoverStop.EXHAUSTED,
            error:   agentError("key revoked"),
          },
        }),
      }),
      null,
    );
    expect(dom).toContain("Moved to openai/gpt-5.4 after 1 attempt");
    expect(dom).toContain("Stopped: routes exhausted");
    expect(dom).toContain("rate limited");
    expect(dom).toContain("key revoked");
  });

  test("renders empty-friendly content when stage projection is missing", () => {
    const dom = render(undefined, null);
    // sidebar still renders even with no data
    expect(dom).toContain("Agent");
    expect(dom).not.toContain("Moved to");
  });
});
