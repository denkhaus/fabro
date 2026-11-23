import { Link, useOutlet, useSearchParams } from "react-router";
import useSWR from "swr";

import { workItemsApi } from "../lib/api-client";
import type { ListWorkItemsSortEnum, ListWorkItemsStateEnum } from "@qltysh/fabro-api-client";
import { DataTable, estimatePageTotal, type DataTableColumn, type DataTableFilter } from "../components/data-table";
import { EmptyState, ErrorState, LoadingState } from "../components/state";
import { Markdown } from "../components/stage-renderers/primitives";
import { labelBadgeClass, prioBadgeClass, trackerItemState, typeBadgeClass, workItemLinkClass } from "../components/badges";

/**
 * The work items view (denkhaus/work#1199): the tracker's read surface
 * in the web app — read-only (mutations stay on the CLI in v1), paged
 * and filtered through the shared data table control (#1201), with the
 * lane/priority/type filters the tracker's labels carry and the status
 * (open/closed/all) door the state param serves (denkhaus/work#1665).
 * The key link colors by the item's own live state (#1298 — the same
 * palette the observation surfaces share).
 */

export const handle = { wide: true };

const LANE_OPTIONS = [
  { value: "loop", label: "loop" },
  { value: "develop", label: "develop" },
  { value: "architect", label: "architect" },
  { value: "conductor", label: "conductor" },
];

const TYPE_OPTIONS = [
  { value: "type:bug", label: "bug" },
  { value: "type:feature", label: "feature" },
  { value: "type:task", label: "task" },
  { value: "type:epic", label: "epic" },
];

interface WorkItemRow {
  id: string;
  key: string;
  title: string;
  priority: string;
  lane: string;
  type: string;
  state: string | null;
  url: string;
  updatedAt: string;
}

function labelValue(labels: string[], prefix: string): string {
  return labels.find((label) => label.startsWith(prefix))?.slice(prefix.length) ?? "—";
}

const COLUMNS: Array<DataTableColumn<WorkItemRow>> = [
  {
    id: "key",
    header: "Key",
    cell: (row) => (
      // The key rides a QUERY parameter (the same decision as the API
      // door): it is opaque (`owner/repo#n`), and a slash inside a path
      // segment is not routable — the router decodes before matching
      // and the segment splits (the live deep link fell back to the
      // list, 2026-11-10).
      <Link
        to={`/work/item?key=${encodeURIComponent(row.key)}`}
        className={`font-mono ${workItemLinkClass(row.state)} hover:underline`}
      >
        {row.key}
      </Link>
    ),
    sortKey: "updated",
  },
  { id: "title", header: "Title", cell: (row) => row.title },
  {
    id: "priority",
    header: "Prio",
    cell: (row) =>
      row.priority === "—" ? (
        ""
      ) : (
        <span className={prioBadgeClass(row.priority)}>{row.priority}</span>
      ),
  },
  {
    id: "type",
    header: "Type",
    cell: (row) =>
      row.type === "—" ? (
        ""
      ) : (
        <span className={typeBadgeClass(row.type)}>{row.type}</span>
      ),
  },
  { id: "lane", header: "Lane", cell: (row) => (row.lane === "—" ? "" : <span className="font-mono">{row.lane}</span>) },
];

const PRIORITY_OPTIONS = [
  { value: "p0", label: "p0" },
  { value: "p1", label: "p1" },
  { value: "p2", label: "p2" },
  { value: "p3", label: "p3" },
];

const STATUS_OPTIONS = [
  { value: "open", label: "open" },
  { value: "closed", label: "closed" },
  { value: "all", label: "all" },
];

// Selects first, free-text Search last — the observations table's
// toolbar order.
const FILTERS: DataTableFilter[] = [
  { key: "lane", label: "Lane", kind: "select", options: LANE_OPTIONS },
  { key: "label", label: "Type", kind: "select", options: TYPE_OPTIONS },
  { key: "priority", label: "Priority", kind: "select", options: PRIORITY_OPTIONS },
  { key: "status", label: "Status", kind: "select", options: STATUS_OPTIONS },
  { key: "query", label: "Search", kind: "text", placeholder: "Search title and body…" },
];

export default function WorkItemsView() {
  // A child route (/work/item?key=…) replaces the list when active —
  // the parent must render the Outlet for the child to appear at all
  // (the detail silently never showed while the list had no Outlet).
  const outlet = useOutlet();
  if (outlet) {
    return outlet;
  }
  return (
    <DataTable<WorkItemRow>
      swrKey="work-items"
      columns={COLUMNS}
      filters={FILTERS}
      emptyLabel="No open work items."
      fetcher={async ({ page, limit, sort, filters }) => {
        // The work listing pages by (page, limit); the pager's total is
        // the tracker's exact count (GitHub search's total_count,
        // denkhaus/work#1296) — the page-window estimate is only the
        // fallback for a server that predates the field.
        const response = await workItemsApi.listWorkItems(
          "fabro",
          filters.query ? undefined : filters.lane,
          filters.query || undefined,
          limit,
          filters.label || undefined,
          // The priority filter rides the prio: label prefix server-side
          // (denkhaus/work#1665) — the same label mechanism as the lane
          // and raw label filters.
          filters.priority || undefined,
          (sort ?? undefined) as ListWorkItemsSortEnum | undefined,
          // The status filter is the state param (denkhaus/work#1665):
          // unset stays the server's open default, "all" is the census
          // read. work#1476 inserted state + cursor before page; the
          // cursor stays unset (the pager walks pages).
          (filters.status || undefined) as ListWorkItemsStateEnum | undefined,
          undefined,
          page,
        );
        const items = response.data?.data ?? [];
        const rows: WorkItemRow[] = items.map((item) => ({
          id: item.itemKey ?? item.url,
          key: item.itemKey ?? "",
          title: item.title ?? "",
          priority: labelValue(item.labels ?? [], "prio:"),
          lane: labelValue(item.labels ?? [], "lane:"),
          type: labelValue(item.labels ?? [], "type:"),
          // work#1511: the raw tracker state joins the shared live-state
          // vocabulary (completed-only closures ARE merged work) so the
          // table's links wear the observations table's per-state colors.
          state: trackerItemState(item.state ?? null),
          url: item.url ?? "",
          updatedAt: item.updatedAt ?? "",
        }));
        const served = response.data?.total;
        const total = served ?? estimatePageTotal(rows.length, page, limit);
        return { rows, total };
      }}
    />
  );
}

export function WorkItemDetail() {
  const [searchParams] = useSearchParams();
  const key = searchParams.get("key");
  // Key-addressed fetch (denkhaus/work#1235): the list's text query
  // searches title and body, so the detail reads its own door. The
  // generated client orders the QUERY param first: (itemKey, projectId).
  const { data: item, isLoading, error } = useSWR(
    ["work-item", key],
    async () => (await workItemsApi.getWorkItem(key ?? "", "fabro")).data ?? null,
  );

  if (isLoading) {
    return <LoadingState label="Loading work item…" />;
  }
  if (error) {
    return <ErrorState title="Work item failed to load" description={String(error)} />;
  }
  if (!item || (item.itemKey && key && item.itemKey !== key)) {
    return (
      <div className="py-12">
        <EmptyState
          title="Work item not found"
          description={`No item matches ${key ?? "this key"}.`}
        />
      </div>
    );
  }
  return (
    <div className="mx-auto max-w-4xl space-y-4">
      <div className="flex flex-wrap items-baseline gap-3">
        <h1 className="text-lg font-medium text-fg">{item.title}</h1>
        <span className="font-mono text-sm text-teal-500">{item.itemKey}</span>
        <Link
          to={`/work?${new URLSearchParams(searchParams).toString()}`}
          className="ml-auto text-xs text-fg-muted hover:text-fg"
        >
          back to the list
        </Link>
      </div>
      <div className="flex flex-wrap gap-1.5">
        {(item.labels ?? []).map((label) => (
          <span key={label} className={labelBadgeClass(label)}>
            {label}
          </span>
        ))}
      </div>
      <div className="rounded-md border border-line bg-panel/60 px-4 py-3 text-sm">
        <Markdown content={item.body ?? ""} />
      </div>
      {item.url && (
        <a
          href={item.url}
          target="_blank"
          rel="noreferrer"
          className="inline-block text-xs text-teal-500 hover:underline"
        >
          open in the tracker ↗
        </a>
      )}
    </div>
  );
}
