import { afterEach, describe, expect, mock, test } from "bun:test";
import TestRenderer from "react-test-renderer";
import { createMemoryRouter, RouterProvider } from "react-router";
import type { GlobalObservationList } from "@qltysh/fabro-api-client";

let currentFeed: GlobalObservationList | undefined;

// Partial-mock (the run-observations test's pattern): spread the REAL
// module so every export stays available to later files in the same
// run; override only what this test needs.
const realApiClient = await import("../lib/api-client");
mock.module("../lib/api-client", () => ({
  ...realApiClient,
  observationsApi: {
    listObservations: async () => ({
      data: currentFeed,
    }),
  },
}));

const { default: ObservationsFeedView } = await import("./observations");

let activeRenderer: TestRenderer.ReactTestRenderer | null = null;

function renderFeed(
  observations: unknown[],
  initialEntry: string,
  total?: number,
): TestRenderer.ReactTestRenderer {
  currentFeed = { observations, total } as GlobalObservationList;
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

  const router = createMemoryRouter([
    { path: "/observations", element: <ObservationsFeedView /> },
    { path: "/runs/:id/observations", element: <div>run observations</div> },
  ], { initialEntries: [initialEntry] });

  let renderer: TestRenderer.ReactTestRenderer | undefined;
  TestRenderer.act(() => {
    renderer = TestRenderer.create(<RouterProvider router={router} />);
  });
  activeRenderer = renderer;
  return renderer!;
}

/// The DataTable reads through SWR; flush its fetch promise so the rows
/// render before assertions.
async function flushRows(renderer: TestRenderer.ReactTestRenderer) {
  await TestRenderer.act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  return renderer;
}

function textFromNode(node: ReturnType<TestRenderer.ReactTestRenderer["toJSON"]>): string {
  if (!node) return "";
  if (typeof node === "string") return node;
  if (Array.isArray(node)) return node.map(textFromNode).join(" ");
  return (node.children ?? []).map(textFromNode).join(" ");
}

describe("ObservationsFeedView", () => {
  afterEach(() => {
    currentFeed = undefined;
    // Unmount AND distinct `?page=` per test: the DataTable's SWR key is
    // fixed otherwise, and SWR dedupes an identical key within two
    // seconds — the next test would read this test's cached rows.
    activeRenderer?.unmount();
    activeRenderer = null;
    delete (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT;
  });

  // denkhaus/work#1235, decision 7: the feed NEVER abbreviates run ids
  // (the old slice to 13 characters is gone).
  test("renders the full run id, never the 13-character slice", async () => {
    const renderer = await flushRows(renderFeed([
      {
        run_id: "01M6SGVZ4V1DABCDEFGH",
        seq: 1,
        category: "painpoint",
        triage_state: "untriaged",
        text: "something hurt",
        recorded_at: "2026-12-11T11:38:44Z",
        linked_work_items: [],
      },
    ], "/observations?page=1"));
    const json = renderer.toJSON();
    const text = textFromNode(json);
    expect(text).toContain("01M6SGVZ4V1DABCDEFGH");
    expect(text).not.toContain("…");
    // The run id links into the run's observations tab.
    expect(JSON.stringify(json)).toContain("/runs/01M6SGVZ4V1DABCDEFGH/observations");
  });

  // denkhaus/work#1296: the pager's total is the server's EXACT count,
  // never the page-window estimate — a short first page over a 25-record
  // feed renders 25 items and 3 pages (the estimate showed 2 items).
  test("renders the served exact total, not a page-window estimate", async () => {
    const renderer = await flushRows(renderFeed([
      {
        run_id: "01M6SGVZ4V1D",
        seq: 1,
        category: "note",
        triage_state: "untriaged",
        text: "one of many",
        recorded_at: "2026-12-11T11:38:44Z",
        linked_work_items: [],
      },
      {
        run_id: "01M6SGVZ4V1E",
        seq: 1,
        category: "painpoint",
        triage_state: "untriaged",
        text: "another of many",
        recorded_at: "2026-12-11T11:39:44Z",
        linked_work_items: [],
      },
    ], "/observations?page=4", 25));
    const text = textFromNode(renderer.toJSON());
    expect(text).toMatch(/25\s+item/);
    // The requested page 4 clamps to the last page of the 25-record feed.
    expect(text).toMatch(/page\s+3\s+\/\s+3/);
  });

  // denkhaus/work#1235, decision 5: absolute `dd.MM.yyyy HH:mm:ss`
  // datetimes, never relative.
  test("renders absolute datetimes with seconds", async () => {
    const before = Date.now();
    const renderer = await flushRows(renderFeed([
      {
        run_id: "01M6SGVZ4V1D",
        seq: 1,
        category: "note",
        triage_state: "filed",
        text: "a note",
        recorded_at: new Date("2026-12-11T10:38:44Z").toISOString(),
        linked_work_items: [],
      },
    ], "/observations?page=2"));
    const text = textFromNode(renderer.toJSON());
    // The rendered datetime is local time assembled from explicit parts;
    // assert the SHAPE (dd.dd.dddd dd:dd:dd), not the zone offset.
    expect(text).toMatch(/\d{2}\.\d{2}\.\d{4} \d{2}:\d{2}:\d{2}/);
    expect(text).not.toContain("ago");
    expect(Date.now() - before).toBeGreaterThanOrEqual(0);
  });

  // denkhaus/work#1235, decision 3: linked items anchor to the
  // server-resolved tracker URL.
  test("linked work items anchor to the server-resolved tracker URL", async () => {
    const renderer = await flushRows(renderFeed([
      {
        run_id: "01M6SGVZ4V1D",
        seq: 2,
        category: "painpoint",
        triage_state: "filed",
        text: "filed an item",
        recorded_at: "2026-12-11T10:38:44Z",
        linked_work_items: [
          { item_key: "denkhaus/work#12", item_state: "open", item_url: "https://tracker.example/issues/12" },
        ],
      },
    ], "/observations?page=3"));
    const json = renderer.toJSON();
    const text = textFromNode(json);
    expect(text).toContain("denkhaus/work#12");
    expect(JSON.stringify(json)).toContain("https://tracker.example/issues/12");
  });
});
