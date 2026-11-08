import { Link } from "react-router";

import { DataTable, type DataTableColumn, type DataTableFilter } from "../components/data-table";
import { observationsApi } from "../lib/api-client";

/**
 * The global observations view (denkhaus/work#1200): every run's
 * observation records in one feed — type and triage state as dropdown
 * filters (user directive 2026-11-08: no checkboxes, no radios), the
 * pager rides the shared data table control (#1201).
 */

interface FeedRow {
  id: string;
  runId: string;
  workflow: string;
  seq: number;
  category: string;
  triage: string;
  text: string;
  recordedAt: string;
  items: string;
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
        {row.runId.slice(0, 13)}…
      </Link>
    ),
  },
  {
    id: "type",
    header: "Type",
    cell: (row) => <span className="font-mono">{row.category}</span>,
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
      row.items ? <span className="font-mono text-fg-3">{row.items}</span> : <span className="text-fg-muted">—</span>,
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

function formatWhen(iso: string | null): string {
  if (!iso) return "—";
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const day = String(date.getDate()).padStart(2, "0");
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  return `${day}.${month}. ${hours}:${minutes}`;
}

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
          recordedAt: formatWhen(item.recorded_at ?? null),
          items: (item.linked_work_items ?? [])
            .map((link) => link.item_key ?? "")
            .filter(Boolean)
            .join(", "),
        }));
        const total = rows.length < limit ? (page - 1) * limit + rows.length : (page + 1) * limit;
        return { rows, total };
      }}
    />
  );
}
