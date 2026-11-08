import { afterEach, describe, expect, mock, test } from "bun:test";
import TestRenderer from "react-test-renderer";
import { MemoryRouter } from "react-router";

const fetchCalls: Array<{ page: number; limit: number; sort: string | null; filters: Record<string, string> }> = [];

mock.module("swr", () => ({
  default: (_key: unknown, fetcher: () => Promise<unknown>) => {
    // The real hook invokes the fetcher; the fake does too (the test
    // fetcher records its arguments synchronously before resolving).
    void fetcher();
    return { data: currentData, isLoading: false, error: undefined };
  },
}));

let currentData: { rows: Array<{ id: string; key: string; title: string }>; total: number } | undefined;

const { DataTable } = await import("./data-table");

function renderTable(initialEntry = "/") {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  let renderer: TestRenderer.ReactTestRenderer | undefined;
  TestRenderer.act(() => {
    renderer = TestRenderer.create(
      <MemoryRouter initialEntries={[initialEntry]}>
        <DataTable
          swrKey="test"
          columns={[
            { id: "key", header: "Key", cell: (row) => row.key, sortKey: "key" },
            { id: "title", header: "Title", cell: (row) => row.title },
          ]}
          filters={[{ key: "lane", label: "Lane", kind: "text" }]}
          fetcher={async (args) => {
            fetchCalls.push(args);
            return currentData ?? { rows: [], total: 0 };
          }}
          emptyLabel="Nothing here."
        />
      </MemoryRouter>,
    );
  });
  return renderer!;
}

function textOf(renderer: TestRenderer.ReactTestRenderer): string {
  const walk = (node: TestRenderer.ReactTestRendererJSON | null | Array<TestRenderer.ReactTestRendererJSON | null> | string): string => {
    if (node == null) return "";
    if (typeof node === "string") return node;
    if (Array.isArray(node)) return node.map(walk).join(" ");
    return (node.children ?? []).map(walk).join(" ");
  };
  return walk(renderer.toJSON());
}

describe("DataTable (denkhaus/work#1201)", () => {
  afterEach(() => {
    fetchCalls.length = 0;
    currentData = undefined;
  });

  test("renders rows and pages with the default items-per-page of 10", async () => {
    currentData = {
      rows: [{ id: "1", key: "denkhaus/work#1", title: "one" }],
      total: 23,
    };
    const renderer = renderTable();
    const text = textOf(renderer);
    expect(text).toContain("denkhaus/work#1");
    expect(text).toMatch(/23\s+item/);
    expect(text).toMatch(/page\s+1\s+\/\s+3/);
    expect(fetchCalls.at(-1)?.limit).toBe(10);
  });

  test("URL params drive page, limit and sort through the fetcher", async () => {
    currentData = { rows: [], total: 0 };
    renderTable("/?page=2&limit=25&sort=-key");
    const last = fetchCalls.at(-1)!;
    expect(last.page).toBe(2);
    expect(last.limit).toBe(25);
    expect(last.sort).toBe("-key");
  });

  test("renders the empty state and names filters when they excluded everything", async () => {
    currentData = { rows: [], total: 0 };
    const plain = renderTable();
    expect(textOf(plain)).toContain("Nothing here.");
    const filtered = renderTable("/?lane=loop");
    expect(textOf(filtered)).toContain("No items match the filters.");
  });

  test("a sortKey column renders the sort button and the active direction marker", async () => {
    currentData = { rows: [{ id: "1", key: "k", title: "t" }], total: 1 };
    const ascending = renderTable("/?sort=key");
    expect(JSON.stringify(ascending.toJSON())).toContain("▲");
    const descending = renderTable("/?sort=-key");
    expect(JSON.stringify(descending.toJSON())).toContain("▼");
  });
});
