import { afterEach, describe, expect, mock, test } from "bun:test";
import TestRenderer from "react-test-renderer";

import type { RunObservationList } from "@qltysh/fabro-api-client";

import type { RunObservationWithStage } from "./run-observations";

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
  observations: RunObservationWithStage[],
): TestRenderer.ReactTestRenderer {
  currentObservations = { observations } as RunObservationList;
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

  let renderer: TestRenderer.ReactTestRenderer | undefined;
  TestRenderer.act(() => {
    renderer = TestRenderer.create(
      <RunObservationsRoute params={{ id: "run_1" }} />,
    );
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

  test("renders stage, type, data, and the linked work item per record", () => {
    const renderer = renderObservations([
      {
        seq: 1,
        category: "painpoint",
        text: "the gate re-ran a cold cache",
        execution: 3,
        firing: 1,
        recorded_at: "2026-11-08T00:00:00Z",
        stage: "reviewer@3",
        linked_work_items: [{ item_key: "denkhaus/work#9" }],
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
    const text = textFromNode(renderer.toJSON());
    expect(text).toContain("Stage");
    expect(text).toContain("Type");
    expect(text).toContain("Data");
    expect(text).toContain("Work item");
    expect(text).toContain("reviewer@3");
    expect(text).toContain("painpoint");
    expect(text).toContain("the gate re-ran a cold cache");
    expect(text).toContain("denkhaus/work#9");
    // A positionless record renders un-attributed with an empty column.
    expect(text).toContain("nothing durable to record");
    expect(text).toContain("—");
  });
});
