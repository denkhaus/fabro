import { useCallback, useMemo } from "react";
import { Link, useSearchParams } from "react-router";
import useSWR from "swr";

/**
 * One column of the data table (denkhaus/work#1201). `sortKey` absent —
 * the column renders without the sort affordance; sorting is a REQUEST
 * parameter (the fetcher carries it), never a client-side re-order of
 * the current page.
 */
export interface DataTableColumn<Row> {
  id: string;
  header: string;
  cell: (row: Row) => React.ReactNode;
  sortKey?: string;
  align?: "left" | "right";
  className?: string;
}

/**
 * One filter the toolbar renders beside the pager: a free-text input or
 * a select; the value rides the URL search params under `key`.
 */
export interface DataTableFilter {
  key: string;
  label: string;
  kind: "text" | "select";
  options?: Array<{ value: string; label: string }>;
  placeholder?: string;
}

/**
 * The fetcher contract: server-paged rows plus the total that pages the
 * pager. `sort` is `<key>` or `-<key>` for descending.
 */
export type DataTableFetcher<Row> = (args: {
  page: number;
  limit: number;
  sort: string | null;
  filters: Record<string, string>;
}) => Promise<{ rows: Row[]; total: number }>;

const PAGE_SIZES = [10, 25, 50, 100];

export function DataTable<Row extends { id: string }>({
  columns,
  filters,
  fetcher,
  emptyLabel,
  swrKey,
}: {
  columns: Array<DataTableColumn<Row>>;
  filters: DataTableFilter[];
  fetcher: DataTableFetcher<Row>;
  emptyLabel: string;
  swrKey: string;
}) {
  const [searchParams, setSearchParams] = useSearchParams();
  const page = Math.max(1, Number(searchParams.get("page") ?? "1") || 1);
  const limit = Number(searchParams.get("limit") ?? "10") || 10;
  const sort = searchParams.get("sort");

  const filterValues = useMemo(() => {
    const values: Record<string, string> = {};
    for (const filter of filters) {
      const value = searchParams.get(filter.key);
      if (value) values[filter.key] = value;
    }
    return values;
  }, [filters, searchParams]);

  const requestArgs = { page, limit, sort, filters: filterValues };
  const { data, isLoading, error } = useSWR(
    [swrKey, page, limit, sort, JSON.stringify(filterValues)],
    () => fetcher(requestArgs),
    { keepPreviousData: true },
  );

  const updateParams = useCallback(
    (mutate: (params: URLSearchParams) => void) => {
      setSearchParams(
        (previous) => {
          const next = new URLSearchParams(previous);
          mutate(next);
          return next;
        },
        { replace: false },
      );
    },
    [setSearchParams],
  );

  const total = data?.total ?? 0;
  const pageCount = Math.max(1, Math.ceil(total / limit));
  const currentPage = Math.min(page, pageCount);
  const windowStart = Math.max(1, Math.min(currentPage - 2, pageCount - 4));
  const windowEnd = Math.min(pageCount, windowStart + 4);

  const setSort = (column: DataTableColumn<Row>) => {
    if (column.sortKey == null) return;
    updateParams((params) => {
      const nextSort = sort === column.sortKey ? `-${column.sortKey}` : column.sortKey;
      params.set("sort", nextSort);
      params.delete("page");
    });
  };

  const rows = data?.rows ?? [];
  const filtered = filters.some((filter) => filterValues[filter.key] != null);

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap items-center gap-2">
        {filters.map((filter) =>
          filter.kind === "text" ? (
            <input
              key={filter.key}
              type="search"
              placeholder={filter.placeholder ?? filter.label}
              aria-label={filter.label}
              defaultValue={filterValues[filter.key] ?? ""}
              onChange={(event) =>
                updateParams((params) => {
                  if (event.target.value) params.set(filter.key, event.target.value);
                  else params.delete(filter.key);
                  params.delete("page");
                })
              }
              className="w-48 rounded-md border border-line bg-panel px-2.5 py-1.5 text-sm text-fg outline-none focus:border-line-strong"
            />
          ) : (
            <select
              key={filter.key}
              aria-label={filter.label}
              value={filterValues[filter.key] ?? ""}
              onChange={(event) =>
                updateParams((params) => {
                  if (event.target.value) params.set(filter.key, event.target.value);
                  else params.delete(filter.key);
                  params.delete("page");
                })
              }
              className="rounded-md border border-line bg-panel px-2.5 py-1.5 text-sm text-fg outline-none focus:border-line-strong"
            >
              <option value="">{filter.label}: all</option>
              {(filter.options ?? []).map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          ),
        )}
        <span className="ml-auto font-mono text-xs text-fg-muted">
          {total} item{total === 1 ? "" : "s"}
        </span>
      </div>

      <div className="overflow-hidden rounded-md border border-line">
        <div className="max-h-[70vh] overflow-y-auto">
          <table className="w-full text-sm">
            <thead className="sticky top-0 z-10 bg-panel">
              <tr className="border-b border-line text-left text-xs font-medium text-fg-3">
                {columns.map((column) => (
                  <th
                    key={column.id}
                    className={`px-4 py-2.5 font-medium ${column.align === "right" ? "text-right" : ""} ${column.className ?? ""}`}
                  >
                    {column.sortKey == null ? (
                      column.header
                    ) : (
                      <button
                        type="button"
                        onClick={() => setSort(column)}
                        className="inline-flex items-center gap-1 hover:text-fg"
                      >
                        {column.header}
                        {sort === column.sortKey && <span aria-hidden>▲</span>}
                        {sort === `-${column.sortKey}` && <span aria-hidden>▼</span>}
                      </button>
                    )}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {isLoading && rows.length === 0
                ? Array.from({ length: 5 }, (_, index) => (
                    <tr key={`skeleton-${index}`} className="border-b border-line last:border-b-0">
                      {columns.map((column) => (
                        <td key={column.id} className="px-4 py-3">
                          <span className="inline-block h-3 w-24 animate-pulse rounded bg-line" />
                        </td>
                      ))}
                    </tr>
                  ))
                : rows.map((row) => (
                    <tr key={row.id} className="border-b border-line align-top last:border-b-0 hover:bg-overlay/40">
                      {columns.map((column) => (
                        <td
                          key={column.id}
                          className={`px-4 py-3 ${column.align === "right" ? "text-right" : ""} ${column.className ?? ""}`}
                        >
                          {column.cell(row)}
                        </td>
                      ))}
                    </tr>
                  ))}
            </tbody>
          </table>
          {rows.length === 0 && !isLoading && (
            <div className="px-4 py-10 text-center text-sm text-fg-muted">
              {filtered ? `No items match the filters.` : emptyLabel}
            </div>
          )}
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-3 font-mono text-xs text-fg-muted">
        <label className="flex items-center gap-1.5">
          <span>/ page</span>
          <select
            aria-label="Items per page"
            value={String(limit)}
            onChange={(event) =>
              updateParams((params) => {
                params.set("limit", event.target.value);
                params.delete("page");
              })
            }
            className="rounded border border-line bg-panel px-1.5 py-1 text-fg"
          >
            {PAGE_SIZES.map((size) => (
              <option key={size} value={size}>
                {size}
              </option>
            ))}
          </select>
        </label>
        {currentPage > 1 && (
          <Link
            to={`?${new URLSearchParams({ ...Object.fromEntries(searchParams), page: String(currentPage - 1) }).toString()}`}
            className="rounded border border-line px-2 py-1 hover:text-fg"
          >
            prev
          </Link>
        )}
        {Array.from({ length: windowEnd - windowStart + 1 }, (_, index) => windowStart + index).map((number) => (
          <Link
            key={number}
            to={`?${new URLSearchParams({ ...Object.fromEntries(searchParams), page: String(number) }).toString()}`}
            className={`rounded px-2 py-1 ${number === currentPage ? "bg-panel text-fg" : "hover:text-fg"}`}
          >
            {number}
          </Link>
        ))}
        {currentPage < pageCount && (
          <Link
            to={`?${new URLSearchParams({ ...Object.fromEntries(searchParams), page: String(currentPage + 1) }).toString()}`}
            className="rounded border border-line px-2 py-1 hover:text-fg"
          >
            next
          </Link>
        )}
        <span>
          page {currentPage} / {pageCount}
        </span>
      </div>
    </div>
  );
}
