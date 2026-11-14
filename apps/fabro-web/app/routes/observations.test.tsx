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

  /// denkhaus/work#1296: the pager pages by the server's REAL total
  /// (a SQL COUNT under the same filters) — a full page with a real
  /// total NEVER grows the count the way the page-window estimate did.
  test("a full page with a real total pages by that total, not the estimate", async () => {
    const fullPage = Array.from({ length: 10 }, (_, index) => ({
      run_id: "01M6SGVZ4V1D",
      seq: index + 1,
      category: "note",
      triage_state: "untriaged",
      text: `note ${index + 1}`,
      recorded_at: "2026-12-11T10:38:44Z",
      linked_work_items: [],
    }));
    // Distinct ?page per test (SWR dedupes an identical key — see the
    // afterEach note); pages 1-3 are taken by the tests above.
    const renderer = await flushRows(renderFeed(fullPage, "/observations?page=5", 12));
    const text = textFromNode(renderer.toJSON()).replace(/\s+/g, "");
    // 12 real observations -> "12 items" and 2 pages (the requested
    // page 5 clamps to the real last page). The estimate would claim
    // (5 + 1) * 10 = 60 — exactly the growth the defect showed (a
    // number appearing that was not there before).
    expect(text).toContain("12items");
    expect(text).toContain("page2/2");
    expect(text).not.toContain("60items");
  });

  /// denkhaus/work#1296: with the triage filter set the server cannot
  /// count honestly (triage derives per row) and reports total: null —
  /// the fetcher falls back to the page-window estimate.
  test("a null total falls back to the page-window estimate", async () => {
    const fullPage = Array.from({ length: 10 }, (_, index) => ({
      run_id: "01M6SGVZ4V1D",
      seq: index + 1,
      category: "painpoint",
      triage_state: "untriaged",
      text: `painpoint ${index + 1}`,
      recorded_at: "2026-12-11T10:38:44Z",
      linked_work_items: [],
    }));
    const renderer = await flushRows(renderFeed(fullPage, "/observations?page=6"));
    const text = textFromNode(renderer.toJSON()).replace(/\s+/g, "");
    // No total: the fallback estimates (6 + 1) * 10 = 70.
    expect(text).toContain("70items");
  });
});
