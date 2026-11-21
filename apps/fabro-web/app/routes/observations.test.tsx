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

/// Collects every anchor in the rendered tree with its className, title
/// and text — the work-item link pins read their state color off it.
function workItemAnchors(node: unknown): Array<{ className?: string; title?: string; text: string }> {
  const found: Array<{ className?: string; title?: string; text: string }> = [];
  function walk(node: unknown): void {
    if (!node || typeof node !== "object") return;
    if (Array.isArray(node)) {
      for (const child of node) walk(child);
      return;
    }
    const element = node as {
      type?: unknown;
      props?: { className?: string; title?: string };
      children?: unknown;
    };
    if (element.type === "a") {
      found.push({
        className: element.props?.className,
        title: element.props?.title,
        text: textFromNode(node as ReturnType<TestRenderer.ReactTestRenderer["toJSON"]>),
      });
    }
    // react-test-renderer JSON nodes keep children at the node level,
    // beside props — not inside them.
    walk(element.children);
  }
  walk(node);
  return found;
}

function anchorFor(renderer: TestRenderer.ReactTestRenderer, key: string): { className?: string; title?: string; text: string } {
  const anchor = workItemAnchors(renderer.toJSON()).find((a) => a.text === key);
  expect(anchor).toBeDefined();
  return anchor!;
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

  // work#1585: the park renders in the untriaged amber family — the
  // board shows the parked state honestly (filing blocked) and the
  // record returns to the untriaged feed after the park TTL.
  test("renders a filing_blocked park in the amber family", async () => {
    const renderer = await flushRows(renderFeed([
      {
        run_id: "01M6SGVZ4V1D",
        seq: 2,
        category: "painpoint",
        triage_state: "filing_blocked",
        text: "filing rate-limited; parks for the TTL",
        recorded_at: "2026-12-11T11:38:44Z",
        linked_work_items: [],
      },
    ], "/observations?page=1"));
    const text = textFromNode(renderer.toJSON());
    expect(text).toContain("filing blocked");
    expect(JSON.stringify(renderer.toJSON())).toContain("border-amber/40");
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

// denkhaus/work#1298: every link to a work item renders color-coded by
// its LIVE state — open amber (demands work), in_progress teal-500
// (active), merged mint (no violet token exists in the theme; the
// closest existing accent stands in), closed fg-muted (settled), null
// plain fg-3. A null state whose read FAILED (item_state_stale) keeps
// the plain color but gains a dotted underline and a tooltip naming
// the degradation — never a false color for an unresolved state.
describe("ObservationsFeedView work-item link state colors", () => {
  afterEach(() => {
    currentFeed = undefined;
    activeRenderer?.unmount();
    activeRenderer = null;
    delete (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT;
  });

  function observationWithLink(itemState: unknown, stale?: boolean) {
    return {
      run_id: "01M6SGVZ4V1D",
      seq: 7,
      category: "painpoint",
      triage_state: "filed",
      text: "the state colors pin",
      recorded_at: "2026-12-11T10:38:44Z",
      linked_work_items: [
        {
          item_key: "denkhaus/work#21",
          item_state: itemState,
          item_state_stale: stale,
          item_url: "https://tracker.example/issues/21",
        },
      ],
    };
  }

  test("open colors the link amber", async () => {
    const renderer = await flushRows(renderFeed([observationWithLink("open")], "/observations?page=11"));
    const anchor = anchorFor(renderer, "denkhaus/work#21");
    expect(anchor.className).toContain("text-amber");
    expect(anchor.className).not.toContain("text-teal-500");
    expect(anchor.title).toBeUndefined();
  });

  test("in_progress keeps the active teal", async () => {
    const renderer = await flushRows(renderFeed([observationWithLink("in_progress")], "/observations?page=12"));
    const anchor = anchorFor(renderer, "denkhaus/work#21");
    expect(anchor.className).toContain("text-teal-500");
    expect(anchor.title).toBeUndefined();
  });

  test("merged colors the link the violet stand-in accent", async () => {
    const renderer = await flushRows(renderFeed([observationWithLink("merged")], "/observations?page=13"));
    const anchor = anchorFor(renderer, "denkhaus/work#21");
    expect(anchor.className).toContain("text-mint");
    expect(anchor.className).not.toContain("text-teal-500");
    expect(anchor.title).toBeUndefined();
  });

  test("closed settles the link to the muted foreground", async () => {
    const renderer = await flushRows(renderFeed([observationWithLink("closed")], "/observations?page=14"));
    const anchor = anchorFor(renderer, "denkhaus/work#21");
    expect(anchor.className).toContain("text-fg-muted");
    expect(anchor.className).not.toContain("text-teal-500");
    expect(anchor.title).toBeUndefined();
  });

  test("a stale null state underlines dotted and names the degradation", async () => {
    const renderer = await flushRows(renderFeed([observationWithLink(null, true)], "/observations?page=15"));
    const anchor = anchorFor(renderer, "denkhaus/work#21");
    expect(anchor.className).toContain("text-fg-3");
    expect(anchor.className).toContain("decoration-dotted");
    expect(anchor.title).toBe("live state stale — tracker read failed");
    expect(anchor.className).not.toContain("text-teal-500");
  });

  test("a null state without staleness stays plain — no false color, no underline", async () => {
    const renderer = await flushRows(renderFeed([observationWithLink(null)], "/observations?page=16"));
    const anchor = anchorFor(renderer, "denkhaus/work#21");
    expect(anchor.className).toContain("text-fg-3");
    expect(anchor.className).not.toContain("decoration-dotted");
    expect(anchor.title).toBeUndefined();
  });
});
