import { afterEach, describe, expect, mock, test } from "bun:test";
import TestRenderer from "react-test-renderer";

import type {
  RunUsage,
  StageTiming,
} from "@qltysh/fabro-api-client";

import { makeUsage } from "../lib/test-fixtures";

function stageTiming(wall_time_ms = 0, inference_time_ms = 0, tool_time_ms = 0): StageTiming {
  return {
    wall_time_ms,
    inference_time_ms,
    tool_time_ms,
    active_time_ms: inference_time_ms + tool_time_ms,
  };
}

let currentUsage: RunUsage | undefined;

mock.module("../lib/queries", () => ({
  useRunUsage: () => ({ data: currentUsage }),
}));

const { default: RunUsageRoute } = await import("./run-usage");

function runUsage(overrides: Partial<RunUsage> = {}): RunUsage {
  return {
    stages: [],
    totals: {
      timing: stageTiming(),
      usage: makeUsage(),
    },
    by_model: [],
    ...overrides,
  };
}

function renderUsage(data: RunUsage): TestRenderer.ReactTestRenderer {
  currentUsage = data;
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

  let renderer: TestRenderer.ReactTestRenderer | undefined;
  TestRenderer.act(() => {
    renderer = TestRenderer.create(<RunUsageRoute params={{ id: "run_1" }} />);
  });
  return renderer!;
}

function textFromNode(node: ReturnType<TestRenderer.ReactTestRenderer["toJSON"]>): string {
  if (!node) return "";
  if (typeof node === "string") return node;
  if (Array.isArray(node)) return node.map(textFromNode).join(" ");
  return (node.children ?? []).map(textFromNode).join(" ");
}

function textFromInstance(node: TestRenderer.ReactTestInstance): string {
  return node.children
    .map((child) => (typeof child === "string" ? child : textFromInstance(child)))
    .join("");
}

describe("RunUsage", () => {
  afterEach(() => {
    currentUsage = undefined;
    delete (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT;
  });

  test("shows a no-model-usage empty state when every stage called no model", () => {
    const renderer = renderUsage(
      runUsage({
        stages: [
          {
            stage: { id: "start", name: "start" },
            model: null,
            usage: makeUsage(),
            timing: stageTiming(),
            state: "succeeded",
          },
          {
            stage: { id: "command", name: "command" },
            model: null,
            usage: makeUsage(),
            timing: stageTiming(61000),
            state: "succeeded",
          },
        ],
        totals: {
          timing: stageTiming(61000),
          usage: makeUsage(),
        },
      }),
    );

    const text = textFromNode(renderer.toJSON());
    expect(text).toContain("No model usage");
    expect(text).toContain("This run didn't call any AI models.");
    expect(text).not.toContain("No stages yet");
    expect(text).not.toContain("By model");
    expect(text).not.toContain("start");
    expect(text).not.toContain("command");
  });

  test("renders mixed LLM and non-LLM rows while counting only LLM rows by model", () => {
    const renderer = renderUsage(
      runUsage({
        stages: [
          {
            stage: { id: "start", name: "start" },
            model: null,
            usage: makeUsage(),
            timing: stageTiming(),
            state: "succeeded",
          },
          {
            stage: { id: "agent", name: "agent" },
            model: {
              provider: "anthropic",
              model_id: "claude-sonnet-4-5",
            },
            usage: makeUsage({ input: 1200, output: 300 }, 240000),
            timing: stageTiming(42000),
            state: "succeeded",
          },
        ],
        totals: {
          timing: stageTiming(42000),
          usage: makeUsage({ input: 1200, output: 300 }, 240000),
        },
        by_model: [
          {
            model: {
              provider: "anthropic",
              model_id: "claude-sonnet-4-5",
            },
            stages: 1,
            usage: makeUsage({ input: 1200, output: 300 }, 240000),
          },
        ],
      }),
    );

    const text = textFromNode(renderer.toJSON());
    expect(text).not.toContain("start");
    expect(text).toContain("agent");
    expect(text).toContain("By model");
    expect(text).toContain("$0.24");
    // A catalog estimate carries no source tag.
    expect(text).not.toContain("reported");

    const footers = renderer.root.findAll((node) => node.type === "tfoot");
    const byModelFooterCells = footers[1].findAll((node) => node.type === "td");
    expect(textFromInstance(byModelFooterCells[1])).toBe("1");
  });

  test("tags a provider-reported cost with its source", () => {
    const reported = makeUsage({ input: 1200, output: 300 }, {
      usd_micros: 240000,
      source: "provider",
    });
    const renderer = renderUsage(
      runUsage({
        stages: [
          {
            stage: { id: "agent", name: "agent" },
            model: { provider: "openrouter", model_id: "kimi-k3" },
            usage: reported,
            timing: stageTiming(42000),
            state: "succeeded",
          },
        ],
        totals: { timing: stageTiming(42000), usage: reported },
        by_model: [
          {
            model: { provider: "openrouter", model_id: "kimi-k3" },
            stages: 1,
            usage: reported,
          },
        ],
      }),
    );

    const text = textFromNode(renderer.toJSON());
    expect(text).toContain("$0.24");
    expect(text).toContain("reported");
  });

  test("says unknown for a total whose cost is unknown, not zero", () => {
    // One stage priced from the catalog, one the catalog could not price:
    // the rows keep their own costs and the total has none.
    const priced = makeUsage({ input: 1200, output: 300 }, 240000);
    const unpriced = makeUsage({ input: 500, output: 50 });
    const total = makeUsage({ input: 1700, output: 350 });
    const renderer = renderUsage(
      runUsage({
        stages: [
          {
            stage: { id: "plan", name: "plan" },
            model: { provider: "openai", model_id: "gpt-5.4" },
            usage: priced,
            timing: stageTiming(1000),
            state: "succeeded",
          },
          {
            stage: { id: "work", name: "work" },
            model: { provider: "openai", model_id: "mystery" },
            usage: unpriced,
            timing: stageTiming(2000),
            state: "succeeded",
          },
        ],
        totals: { timing: stageTiming(3000), usage: total },
        by_model: [
          { model: { provider: "openai", model_id: "gpt-5.4" }, stages: 1, usage: priced },
          { model: { provider: "openai", model_id: "mystery" }, stages: 1, usage: unpriced },
        ],
      }),
    );

    const text = textFromNode(renderer.toJSON());
    expect(text).toContain("$0.24");
    expect(text).toContain("unknown");
    expect(text).not.toContain("$0.00");

    const footers = renderer.root.findAll((node) => node.type === "tfoot");
    const footerCells = footers[0].findAll((node) => node.type === "td");
    expect(textFromInstance(footerCells[4])).toBe("unknown");
  });

  test("keeps the empty state for runs with no stages", () => {
    const renderer = renderUsage(runUsage());

    const text = textFromNode(renderer.toJSON());
    expect(text).toContain("No stages yet");
    expect(text).toContain("Stages will appear as soon as the run starts executing.");
  });

  test("renders an in-flight row with live usage and includes its elapsed time in the footer", () => {
    const originalNow = Date.now;
    // Pin "now" to 30s after the in-flight row started.
    const startedAt = "2026-04-29T12:00:00.000Z";
    const fakeNow = new Date("2026-04-29T12:00:30.000Z").getTime();
    Date.now = () => fakeNow;

    try {
      const renderer = renderUsage(
        runUsage({
          stages: [
            {
              stage: { id: "in-flight", name: "in-flight" },
              model: {
                provider: "anthropic",
                model_id: "claude-opus-4-6",
                speed: "fast",
              },
              usage: makeUsage({ input: 1200, output: 300 }, 240000),
              timing: stageTiming(),
              started_at: startedAt,
              state: "running",
            },
          ],
          totals: {
            timing: stageTiming(),
            usage: makeUsage({ input: 1200, output: 300 }, 240000),
          },
          by_model: [
            {
              model: {
                provider: "anthropic",
                model_id: "claude-opus-4-6",
                speed: "fast",
              },
              stages: 1,
              usage: makeUsage({ input: 1200, output: 300 }, 240000),
            },
          ],
        }),
      );

      const text = textFromNode(renderer.toJSON());
      // Empty-state must NOT show — the table should appear as soon as the
      // first stage starts.
      expect(text).not.toContain("No stages yet");
      expect(text).toContain("in-flight");
      expect(text).toContain("anthropic:claude-opus-4-6 · fast");
      expect(text).toContain("1.2k");
      expect(text).toContain("0.3k");
      expect(text).toContain("$0.24");
      expect(text).toContain("By model");

      // Both the row's runtime cell and the footer total should reflect
      // ~30s elapsed since started_at.
      expect(text).toContain("30s");

      const footers = renderer.root.findAll((node) => node.type === "tfoot");
      const footerCells = footers[0].findAll((node) => node.type === "td");
      // The Run time column in the footer is index 3 (Total / [empty Model] /
      // Tokens / Run time / Cost).
      const footerRuntime = textFromInstance(footerCells[3]);
      expect(footerRuntime).toContain("30s");
    } finally {
      Date.now = originalNow;
    }
  });
});
