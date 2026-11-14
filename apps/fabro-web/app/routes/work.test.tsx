import { afterEach, describe, expect, mock, test } from "bun:test";
import TestRenderer from "react-test-renderer";
import { createMemoryRouter, RouterProvider } from "react-router";

import type { WorkItem } from "@qltysh/fabro-api-client";

let currentItem: WorkItem | null = null;
let listResult: { data: WorkItem[]; total: number | null } | undefined;

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
    listWorkItems: async () => ({
      data: { data: listResult?.data ?? [], total: listResult?.total ?? null },
    }),
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
