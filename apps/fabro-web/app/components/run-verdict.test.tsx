import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import TestRenderer, { act } from "react-test-renderer";
import { MemoryRouter, Route, Routes } from "react-router";

import { setupReactTestEnv } from "../lib/test-utils";

// denkhaus/work#1312: a guard-parked run (`human_needed` with NO pending
// question — the deadlock-exit park) must still get the verdict bar's full
// steering: retry, retry on latest, fork, archive. The park carries no
// answer door by design; the verdict actions are the answer door.

let currentRun: Record<string, unknown> | undefined;

const realQueries = await import("../lib/queries");
mock.module("../lib/queries", () => ({
  ...realQueries,
  useRun: () => ({ data: currentRun }),
}));

const retryRunMock = mock(() => Promise.resolve({ id: "run-next" }));
const retryLatestRunMock = mock(() =>
  Promise.resolve({ new_run_id: "run-latest" }),
);
const archiveRunMock = mock(() => Promise.resolve({}));
const realRunActions = await import("../lib/run-actions");
mock.module("../lib/run-actions", () => ({
  ...realRunActions,
  retryRun: retryRunMock,
  retryLatestRun: retryLatestRunMock,
  archiveRun: archiveRunMock,
}));

const { RunVerdict } = await import("./run-verdict");

const mountedRenderers: TestRenderer.ReactTestRenderer[] = [];
let teardownReactEnv: (() => void) | undefined;

/** A guard-parked run: human_needed, no question, no failure conclusion. */
function guardParkedRun(): Record<string, unknown> {
  return {
    id: "run-1",
    lifecycle: {
      status: { kind: "human_needed" },
      error: null,
    },
    current_question: null,
  };
}

function textOf(renderer: TestRenderer.ReactTestRenderer): string {
  return JSON.stringify(renderer.toJSON());
}

function renderVerdict(): TestRenderer.ReactTestRenderer {
  let renderer!: TestRenderer.ReactTestRenderer;
  act(() => {
    renderer = TestRenderer.create(
      <MemoryRouter initialEntries={["/runs/run-1"]}>
        <Routes>
          <Route path="/runs/:id" element={<RunVerdict runId="run-1" />} />
        </Routes>
      </MemoryRouter>,
    );
  });
  mountedRenderers.push(renderer);
  return renderer;
}

beforeEach(() => {
  teardownReactEnv = setupReactTestEnv();
  currentRun = undefined;
  retryRunMock.mockClear();
  retryLatestRunMock.mockClear();
  archiveRunMock.mockClear();
});

afterEach(() => {
  for (const renderer of mountedRenderers.splice(0)) {
    act(() => renderer.unmount());
  }
  teardownReactEnv?.();
  teardownReactEnv = undefined;
});

describe("RunVerdict", () => {
  test("a guard-parked run (no question, no recorded failure) still offers all four verdict actions", () => {
    currentRun = guardParkedRun();
    const renderer = renderVerdict();
    const rendered = textOf(renderer);

    expect(rendered).toContain("needs a human decision");
    expect(rendered).toContain("No reason was recorded.");
    for (const action of ["Retry", "Retry on latest", "Fork", "Archive"]) {
      expect(rendered).toContain(action);
    }
  });

  // denkhaus/work#1337: a guard-parked run WITH a recorded stop reason
  // (`lifecycle.error.message` — the schema shape) must render that
  // reason string AND the full four-action steering. The old fixture
  // only pinned the null-reason fallback, so the ghost
  // `conclusionFailure` read never failed a test.
  test("a guard-parked run with a recorded lifecycle.error.message renders the reason and all four actions", () => {
    currentRun = {
      ...guardParkedRun(),
      lifecycle: {
        status: { kind: "human_needed" },
        error: { message: "parked by guard: budget exhausted" },
      },
    };
    const renderer = renderVerdict();
    const rendered = textOf(renderer);

    expect(rendered).toContain("needs a human decision");
    expect(rendered).toContain("parked by guard: budget exhausted");
    expect(rendered).not.toContain("No reason was recorded.");
    for (const action of ["Retry", "Retry on latest", "Fork", "Archive"]) {
      expect(rendered).toContain(action);
    }
  });

  test("the parked shape records no answer door: no interview submit wiring is reached", () => {
    currentRun = guardParkedRun();
    const renderer = renderVerdict();
    // The verdict is the steering surface; nothing in it posts an answer.
    expect(textOf(renderer)).not.toContain("answer");
  });

  test("retry on the parked run calls the retry action and navigates to the successor", async () => {
    currentRun = guardParkedRun();
    const renderer = renderVerdict();
    const retry = renderer.root.findAll(
      (node) => typeof node.props?.onClick === "function",
    )[0];
    await act(async () => {
      retry.props.onClick();
    });
    expect(retryRunMock).toHaveBeenCalledTimes(1);
  });

  test("an active run renders no verdict bar", () => {
    currentRun = {
      id: "run-1",
      lifecycle: { status: { kind: "running" } },
    };
    const renderer = renderVerdict();
    expect(renderer.toJSON()).toBeNull();
  });
});
