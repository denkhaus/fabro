import { afterEach, describe, expect, mock, test } from "bun:test";
import TestRenderer from "react-test-renderer";
import { createMemoryRouter, RouterProvider } from "react-router";

import type { RunObservationList } from "@qltysh/fabro-api-client";

let currentObservations: RunObservationList | undefined;

// Partial-mock: bun's mock.module is process-wide (restore does not
// undo it) — spread the REAL module so every export stays available
// to later files in the same run; override only what this test needs.
const realQueries = await import("../lib/queries");
mock.module("../lib/queries", () => ({
  ...realQueries,
  useRunObservations: () => ({ data: currentObservations, isLoading: false }),
}));

const { default: RunObservationsRoute } = await import("./run-observations");

function renderObservations(
  observations: unknown[],
): TestRenderer.ReactTestRenderer {
  currentObservations = { observations } as RunObservationList;
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

  const router = createMemoryRouter([
    { path: "/runs/:id/observations", element: <RunObservationsRoute /> },
    { path: "/runs/:id/stages/:stageId", element: <div>stage view</div> },
  ], { initialEntries: ["/runs/run_1/observations"] });

  let renderer: TestRenderer.ReactTestRenderer | undefined;
  TestRenderer.act(() => {
    renderer = TestRenderer.create(<RouterProvider router={router} />);
  });
  return renderer!;
}

function textFromNode(node: ReturnType<TestRenderer.ReactTestRenderer["toJSON"]>): string {
  if (!node) return "";
  if (typeof node === "string") return node;
  if (Array.isArray(node)) return node.map(textFromNode).join(" ");
  return (node.children ?? []).map(textFromNode).join(" ");
}

describe("RunObservations", () => {
  afterEach(() => {
    currentObservations = undefined;
    delete (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT;
  });

  test("renders an empty state when the run recorded nothing", () => {
    const renderer = renderObservations([]);
    const text = textFromNode(renderer.toJSON());
    expect(text).toContain("No observations");
  });

  test("renders stage, type, triage state, data, and the linked work item per record", () => {
    const renderer = renderObservations([
      {
        seq: 1,
        category: "painpoint",
        text: "the gate re-ran a cold cache",
        execution: 3,
        firing: 1,
        recorded_at: "2026-11-08T00:00:00Z",
        stage: "reviewer@3",
        linked_work_items: [{ item_key: "denkhaus/work#9", item_state: "in_progress" }],
      },
      {
        seq: 2,
        category: "note",
        text: "nothing durable to record",
        execution: 9,
        firing: 4,
        recorded_at: "2026-11-08T00:01:00Z",
        stage: null,
        linked_work_items: [],
      },
    ]);
    const json = renderer.toJSON();
    const text = textFromNode(json);
    expect(text).toContain("Stage");
    expect(text).toContain("Type");
    expect(text).toContain("State");
    expect(text).toContain("reviewer@3");
    expect(text).toContain("the gate re-ran a cold cache");
    expect(text).toContain("denkhaus/work#9");
    expect(text).toContain("in progress");
    // The triage badge of a linked, in-progress painpoint (the badge
    // renders uppercase through CSS; the text content stays lowercase).
    expect(text).toContain("in progress");
    // The dropdown filters default to type=painpoint + status=open (the
    // old open-painpoints default), so the note leaves the visible set.
    expect(text).toMatch(/1\s+of\s+2\s+observations/);
    expect(text).toContain("type: painpoint");
    expect(text).toContain("status: open");
    // The stage cell links into the run's stage view.
    const stageLink = JSON.stringify(json).includes("/stages/reviewer%403");
    expect(stageLink).toBe(true);
  });

  test("open painpoints filter keeps untriaged and open painpoints, drops closed and notes", () => {
    const renderer = renderObservations([
      {
        seq: 1,
        category: "painpoint",
        text: "untriaged painpoint",
        execution: 1,
        firing: 1,
        recorded_at: "2026-11-08T00:00:00Z",
        stage: null,
        linked_work_items: [],
      },
      {
        seq: 2,
        category: "painpoint",
        text: "closed painpoint",
        execution: 2,
        firing: 1,
        recorded_at: "2026-11-08T00:01:00Z",
        stage: null,
        linked_work_items: [{ item_key: "denkhaus/work#9", item_state: "closed" }],
      },
      {
        seq: 3,
        category: "note",
        text: "plain note",
        execution: 3,
        firing: 1,
        recorded_at: "2026-11-08T00:02:00Z",
        stage: null,
        linked_work_items: [],
      },
    ]);
    const text = textFromNode(renderer.toJSON());
    expect(text).toContain("untriaged painpoint");
    expect(text).toContain("untriaged");
    expect(text).not.toContain("closed painpoint");
    expect(text).not.toContain("plain note");
    expect(text).toMatch(/1\s+of\s+3\s+observations/);
  });
});
