import { Link } from "react-router";

import { DataTable, estimatePageTotal, type DataTableColumn, type DataTableFilter } from "../components/data-table";
import { observationsApi } from "../lib/api-client";
import { formatAbsoluteDateTime } from "../lib/format";
import { categoryBadgeClass } from "../components/badges";

/**
 * The global observations view (denkhaus/work#1200): every run's
 * observation records in one feed — type and triage state as dropdown
 * filters (user directive 2026-11-08: no checkboxes, no radios), the
 * pager rides the shared data table control (#1201). Table refinements
 * (denkhaus/work#1235): full run ids, absolute datetimes, the
 * server-resolved tracker URLs on linked items. The pager's total is
 * the server's EXACT count under the active filters (denkhaus/work#1296)
 * — the page-window estimate is only the skew fallback for an older
 * server.
 */

export const handle = { wide: true };

interface FeedRow {
  id: string;
  runId: string;
  workflow: string;
  seq: number;
  category: string;
  triage: string;
  text: string;
  recordedAt: string;
  items: Array<{ key: string; url: string | null }>;
}

const TRIAGE_STYLE: Record<string, string> = {
  untriaged: "border-amber/40 text-amber",
  filed: "border-line text-fg-3",
  in_progress: "border-teal-500/40 text-teal-500",
  closed: "border-line text-fg-muted",
};

const COLUMNS: Array<DataTableColumn<FeedRow>> = [
  {
    id: "recorded",
    header: "When",
    cell: (row) => <span className="font-mono text-fg-muted">{row.recordedAt}</span>,
    sortKey: "recorded",
  },
  {
    id: "run",
    header: "Run",
    cell: (row) => (
      <Link
        to={`/runs/${row.runId}/observations`}
        className="font-mono text-teal-500 hover:underline"
      >
        {row.runId}
      </Link>
    ),
  },
  {
    id: "type",
    header: "Type",
    cell: (row) => <span className={categoryBadgeClass(row.category)}>{row.category}</span>,
  },
  {
    id: "state",
    header: "State",
    cell: (row) => (
      <span
        className={`rounded-full border px-1.5 py-0.5 font-mono text-[11px] uppercase tracking-wide ${TRIAGE_STYLE[row.triage] ?? "border-line text-fg-muted"}`}
      >
        {row.triage.replace("_", " ")}
      </span>
    ),
  },
  {
    id: "text",
    header: "Observation",
    cell: (row) => (
      <span className="line-clamp-2 whitespace-pre-wrap text-fg-2">{row.text}</span>
    ),
  },
  {
    id: "items",
    header: "Work items",
    cell: (row) =>
      row.items.length ? (
        <span className="font-mono text-fg-3">
          {row.items.map((item, index) => (
            <span key={item.key}>
              {index > 0 && ", "}
              {item.url ? (
                <a
                  href={item.url}
                  target="_blank"
                  rel="noreferrer"
                  className="text-teal-500 hover:underline"
                >
                  {item.key}
                </a>
              ) : (
                item.key
              )}
            </span>
          ))}
        </span>
      ) : (
        <span className="text-fg-muted">—</span>
      ),
  },
];

const FILTERS: DataTableFilter[] = [
  {
    key: "category",
    label: "Type",
    kind: "select",
    options: [
      { value: "painpoint", label: "painpoint" },
      { value: "note", label: "note" },
    ],
  },
  {
    key: "triage",
    label: "Status",
    kind: "select",
    options: [
      { value: "untriaged", label: "untriaged" },
      { value: "filed", label: "filed" },
      { value: "in_progress", label: "in progress" },
      { value: "closed", label: "closed" },
    ],
  },
  { key: "q", label: "Search", kind: "text", placeholder: "Text contains…" },
];

export default function ObservationsFeedView() {
  return (
    <DataTable<FeedRow>
      swrKey="observations-feed"
      columns={COLUMNS}
      filters={FILTERS}
      emptyLabel="No observations recorded yet."
      fetcher={async ({ page, limit, sort, filters }) => {
        const response = await observationsApi.listObservations(
          (filters.category as "painpoint" | "note") ?? undefined,
          (filters.triage as "untriaged" | "filed" | "in_progress" | "closed") ?? undefined,
          undefined,
          filters.q || undefined,
          (sort === "recorded" ? "recorded" : "-recorded") as "-recorded" | "recorded",
          limit,
          page,
        );
        const observations = response.data?.observations ?? [];
        const rows: FeedRow[] = observations.map((item) => ({
          id: `${item.run_id}-${item.seq}`,
          runId: item.run_id ?? "",
          workflow: item.workflow_name ?? "",
          seq: item.seq ?? 0,
          category: item.category ?? "",
          triage: item.triage_state ?? "untriaged",
          text: item.text ?? "",
          recordedAt: formatAbsoluteDateTime(item.recorded_at ?? null),
          items: (item.linked_work_items ?? [])
            .map((link) => ({
              key: link.item_key ?? "",
              url: link.item_url ?? null,
            }))
            .filter((linked) => linked.key),
        }));
        // The pager's total is the server's EXACT count under the active
        // filters (denkhaus/work#1296) — never a page-window guess. The
        // estimate stays only as the fallback for a server (or generated
        // client) that predates the field.
        const served = (response.data as { total?: number } | undefined)?.total;
        const total = served ?? estimatePageTotal(rows.length, page, limit);
        return { rows, total };
      }}
    />
  );
}
