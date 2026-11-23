import { afterEach, describe, expect, mock, test } from "bun:test";
import TestRenderer from "react-test-renderer";
import { createMemoryRouter, RouterProvider } from "react-router";

import type { WorkItem } from "@qltysh/fabro-api-client";

let currentItem: WorkItem | null = null;
let listResult: { data: WorkItem[]; total: number | null } | undefined;
// The denkhaus/work#1665 wiring probe: the list call's filter params,
// captured for the fetcher-wiring assertions.
let listCall: { priority?: string; state?: string } | null = null;

// Partial-mock (the run-observations test's pattern): spread the REAL
// module so every export stays available to later files in the same
// run; override only the detail read and the list read.
const realApiClient = await import("../lib/api-client");
mock.module("../lib/api-client", () => ({
  ...realApiClient,
  workItemsApi: {
    ...realApiClient.workItemsApi,
    getWorkItem: async (itemKey: string, _project: string) => ({
      data:
        currentItem && currentItem.itemKey === itemKey
          ? currentItem
          : null,
    }),
    listWorkItems: async (
      _project: string,
      _lane?: string,
      _query?: string,
      _limit?: number,
      _label?: string,
      priority?: string,
      _sort?: unknown,
      state?: string,
    ) => {
      listCall = { priority, state };
      return { data: { data: listResult?.data ?? [], total: listResult?.total ?? null } };
    },
  },
}));

// The Markdown renderer draws through dangerouslySetInnerHTML —
// invisible to react-test-renderer's tree. Render the content as plain
// text here; the markdown formatting itself is the primitives'
// tested surface.
mock.module("../components/stage-renderers/primitives", () => ({
  Markdown: ({ content }: { content: string }) => <div>{content}</div>,
}));

const { WorkItemDetail, default: WorkItemsView } = await import("./work");

function renderDetail(item: WorkItem | null): TestRenderer.ReactTestRenderer {
  currentItem = item;
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

  const router = createMemoryRouter([
    { path: "/work/item", element: <WorkItemDetail /> },
  ], { initialEntries: ["/work/item?key=denkhaus%2Fwork%2312"] });

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

const ITEM: WorkItem = {
  itemKey: "denkhaus/work#12",
  title: "The collector inlines dockerfile paths",
  body: "Persist-time inlining lands the text inline.",
  state: "open",
  labels: ["project:fabro", "prio:p1", "type:bug"],
  priority: 1,
  url: "https://tracker.example/issues/12",
  updatedAt: "2026-11-10T09:00:00Z",
};

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

/// The detail reads through SWR; flush its fetch promise before
/// assertions.
async function flushDetail(renderer: TestRenderer.ReactTestRenderer) {
  await TestRenderer.act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  return renderer;
}

describe("WorkItemDetail", () => {
  afterEach(() => {
    currentItem = null;
    delete (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT;
  });

  // denkhaus/work#1235, decision 4: the detail link renders — the
  // key-addressed fetch replaces the text-query lookup that could
  // never find an item by its key.
  test("renders the item for its key", async () => {
    const renderer = await flushDetail(renderDetail(ITEM));
    const text = textFromNode(renderer.toJSON());
    expect(text).toContain("The collector inlines dockerfile paths");
    expect(text).toContain("denkhaus/work#12");
    expect(text).toContain("Persist-time inlining lands the text inline.");
    expect(text).toContain("prio:p1");
  });

  test("renders the not-found state when no item answers the key", async () => {
    // A different key: SWR's global cache would replay the first test's
    // entry for the same key otherwise.
    const router = createMemoryRouter([
      { path: "/work/item", element: <WorkItemDetail /> },
    ], { initialEntries: ["/work/item?key=denkhaus%2Fwork%2399"] });
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    let renderer: TestRenderer.ReactTestRenderer | undefined;
    TestRenderer.act(() => {
      renderer = TestRenderer.create(<RouterProvider router={router} />);
    });
    await flushDetail(renderer!);
    const text = textFromNode(renderer!.toJSON());
    expect(text).toContain("Work item not found");
  });
});

describe("WorkItemsView", () => {
  afterEach(() => {
    listResult = undefined;
    delete (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT;
  });

  // denkhaus/work#1296: the pager's total is the tracker's served
  // count (GitHub search's total_count), never the page-window
  // estimate — a 2-row first page over 12 open items renders 12 items
  // and 2 pages (the estimate rendered 2 items and 1 page).
  test("renders the served exact total, not a page-window estimate", async () => {
    listResult = {
      data: [
        { ...ITEM, itemKey: "denkhaus/work#1" },
        { ...ITEM, itemKey: "denkhaus/work#2" },
      ],
      total: 12,
    };
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const router = createMemoryRouter([
      { path: "/work", element: <WorkItemsView /> },
      { path: "/work/item", element: <div>detail</div> },
    ], { initialEntries: ["/work"] });
    let renderer: TestRenderer.ReactTestRenderer | undefined;
    TestRenderer.act(() => {
      renderer = TestRenderer.create(<RouterProvider router={router} />);
    });
    await TestRenderer.act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    const text = textFromNode(renderer!.toJSON());
    expect(text).toMatch(/12\s+item/);
    expect(text).toMatch(/page\s+1\s+\/\s+2/);
  });
});

// denkhaus/work#1298: the work view's item table colors the key link
// by the item's own live state — the same palette the observation
// surfaces share (open amber, in_progress teal-500, merged mint as the
// violet stand-in, closed fg-muted; anything unresolved stays plain
// fg-3 — never a false color). Each test rides a distinct `?page=` so
// SWR's two-second key dedupe can't replay another test's rows.
// denkhaus/work#1665: the priority and status filters wire into the
// list call — priority rides the new `priority` param, status rides
// the `state` param. Unset filters stay undefined, so the default
// table keeps the server's open-only answer.
describe("WorkItemsView filter wiring", () => {
  afterEach(() => {
    listResult = undefined;
    listCall = null;
    delete (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT;
  });

  async function renderListWithFilters(query: string, page: number): Promise<TestRenderer.ReactTestRenderer> {
    listResult = { data: [{ ...ITEM, itemKey: "denkhaus/work#41" }], total: 1 };
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const router = createMemoryRouter([
      { path: "/work", element: <WorkItemsView /> },
      { path: "/work/item", element: <div>detail</div> },
    ], { initialEntries: [`/work?${query}&page=${page}`] });
    let renderer: TestRenderer.ReactTestRenderer | undefined;
    TestRenderer.act(() => {
      renderer = TestRenderer.create(<RouterProvider router={router} />);
    });
    await TestRenderer.act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    return renderer!;
  }

  test("priority and status filters reach the list call", async () => {
    const renderer = await renderListWithFilters("priority=p1&status=closed", 7);
    expect(textFromNode(renderer.toJSON())).toContain("denkhaus/work#41");
    expect(listCall).toEqual({ priority: "p1", state: "closed" });
  });

  test("unset filters stay undefined — the server's open default answers", async () => {
    const renderer = await renderListWithFilters("", 8);
    expect(textFromNode(renderer.toJSON())).toContain("denkhaus/work#41");
    expect(listCall).toEqual({ priority: undefined, state: undefined });
  });
});

describe("WorkItemsView key link state colors", () => {
  afterEach(() => {
    listResult = undefined;
    delete (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT;
  });

  async function renderListWithState(state: WorkItem["state"], page: number): Promise<TestRenderer.ReactTestRenderer> {
    listResult = { data: [{ ...ITEM, itemKey: "denkhaus/work#41", state }], total: 1 };
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    const router = createMemoryRouter([
      { path: "/work", element: <WorkItemsView /> },
      { path: "/work/item", element: <div>detail</div> },
    ], { initialEntries: [`/work?page=${page}`] });
    let renderer: TestRenderer.ReactTestRenderer | undefined;
    TestRenderer.act(() => {
      renderer = TestRenderer.create(<RouterProvider router={router} />);
    });
    await TestRenderer.act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 20));
    });
    return renderer!;
  }

  test("open colors the key link amber", async () => {
    const renderer = await renderListWithState("open", 2);
    const anchor = anchorFor(renderer, "denkhaus/work#41");
    expect(anchor.className).toContain("text-amber");
    expect(anchor.className).not.toContain("text-teal-500");
  });

  test("in_progress keeps the active teal", async () => {
    const renderer = await renderListWithState("in_progress", 3);
    const anchor = anchorFor(renderer, "denkhaus/work#41");
    expect(anchor.className).toContain("text-teal-500");
  });

  test("merged colors the key link the violet stand-in accent", async () => {
    const renderer = await renderListWithState("merged", 4);
    const anchor = anchorFor(renderer, "denkhaus/work#41");
    expect(anchor.className).toContain("text-mint");
    expect(anchor.className).not.toContain("text-teal-500");
  });

  test("closed wears the same done-mint the observations table paints (work#1511)", async () => {
    // The tracker closes items COMPLETED-ONLY, so a closed item is the
    // same landed-work state the enriched vocabulary calls merged —
    // the two tables agree per state, application-wide.
    const renderer = await renderListWithState("closed", 5);
    const anchor = anchorFor(renderer, "denkhaus/work#41");
    expect(anchor.className).toContain("text-mint");
    expect(anchor.className).not.toContain("text-teal-500");
    expect(anchor.className).not.toContain("text-fg-muted");
  });

  test("an unresolved state stays plain — no false color", async () => {
    // A value outside the live-state palette (a missing field lands on
    // the same plain fallback through the route's `?? null` seam).
    const renderer = await renderListWithState("blocked", 6);
    const anchor = anchorFor(renderer, "denkhaus/work#41");
    expect(anchor.className).toContain("text-fg-3");
    expect(anchor.className).not.toContain("text-teal-500");
    expect(anchor.className).not.toContain("decoration-dotted");
  });
});
