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
    // User decision 2026-11-10: the dropdown filters default to
    // type=all + status=all — everything visible, the note included.
    expect(text).toMatch(/2\s+of\s+2\s+observations/);
    expect(text).toContain("type: all");
    expect(text).toContain("status: all");
    // The stage cell links into the run's stage view.
    const stageLink = JSON.stringify(json).includes("/stages/reviewer%403");
    expect(stageLink).toBe(true);
  });

  test("everything stays visible under the all/all defaults", () => {
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
    // User decision 2026-11-10: the filters narrow, they never surprise —
    // untriaged, closed, and notes all render until a filter is chosen.
    expect(text).toContain("untriaged painpoint");
    expect(text).toContain("closed painpoint");
    expect(text).toContain("plain note");
    expect(text).toMatch(/3\s+of\s+3\s+observations/);
  });

  // denkhaus/work#1235: the tracker URL resolves server-side — the
  // linked key anchors to the tracker, never a client-built forge URL;
  // a null URL keeps plain text.
  test("linked items anchor to the server-resolved tracker URL", () => {
    function keysInsideAnchors(node: unknown, insideAnchor: boolean, found: Set<string>): void {
      if (typeof node === "string") {
        if (insideAnchor) found.add(node);
        return;
      }
      if (!node || typeof node !== "object") return;
      if (Array.isArray(node)) {
        for (const child of node) keysInsideAnchors(child, insideAnchor, found);
        return;
      }
      const element = node as {
        type?: string;
        props?: { href?: string };
        children?: unknown;
      };
      const isAnchor = insideAnchor || element.type === "a";
      const href = element.props?.href;
      if (isAnchor && typeof href === "string") found.add(href);
      keysInsideAnchors(element.children, isAnchor, found);
    }

    const renderer = renderObservations([
      {
        seq: 1,
        category: "painpoint",
        text: "linked twice",
        execution: 1,
        firing: 1,
        recorded_at: "2026-11-08T00:00:00Z",
        stage: null,
        linked_work_items: [
          { item_key: "denkhaus/work#12", item_state: "open", item_url: "https://tracker.example/issues/12" },
          { item_key: "denkhaus/work#13", item_state: null, item_url: null },
        ],
      },
    ]);
    const json = renderer.toJSON();
    const text = textFromNode(json);
    expect(text).toContain("denkhaus/work#12");
    expect(text).toContain("denkhaus/work#13");
    const anchored = new Set<string>();
    keysInsideAnchors(json, false, anchored);
    expect(anchored.has("https://tracker.example/issues/12")).toBe(true);
    expect(anchored.has("denkhaus/work#12")).toBe(true);
    expect(anchored.has("denkhaus/work#13")).toBe(false);
  });
});
