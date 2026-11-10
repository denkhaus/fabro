import { Link, useParams } from "react-router";
import type {
  ObservationWorkItemLink,
  RunObservation,
} from "@qltysh/fabro-api-client";
import { useMemo, useState } from "react";

type TypeFilter = "all" | "painpoint" | "note";
type StatusFilter = "all" | "open" | "untriaged" | "filed" | "in_progress" | "closed";

import { EmptyState, ErrorState, LoadingState } from "../components/state";
import { useRunObservations } from "../lib/queries";

const EMPTY_VALUE = "—";

/**
 * One observation's derived triage state: the model the observations
 * surfaces share (the run tab and the global feed). UNTRIAGED — no work
 * item was filed from it (includes merger and adjudicator painpoints
 * that ended in no item); IN_PROGRESS — a linked item is open with a
 * live claim; FILED — linked and open; CLOSED — every linked item is
 * closed.
 */
type TriageState = "untriaged" | "filed" | "in_progress" | "closed";

function leadingLinkState(links: Array<ObservationWorkItemLink>): TriageState {
  if (!links.length) return "untriaged";
  const states = links.map((link) => link.item_state ?? null);
  if (states.some((state) => state === "in_progress")) return "in_progress";
  if (states.some((state) => state === null || state === "open")) return "filed";
  return "closed";
}

const TRIAGE_BADGE_CLASS: Record<TriageState, string> = {
  untriaged: "border-amber/40 text-amber",
  filed: "border-line text-fg-3",
  in_progress: "border-teal-500/40 text-teal-500",
  closed: "border-line text-fg-muted",
};

const TRIAGE_LABEL: Record<TriageState, string> = {
  untriaged: "untriaged",
  filed: "filed",
  in_progress: "in progress",
  closed: "closed",
};

const ITEM_STATE_LABEL: Record<string, string> = {
  open: "open",
  in_progress: "in progress",
  closed: "closed",
};

// #1235: the shell escape stays (the tab's own container bounds at
// 96rem like the other two tables); the old own `max-w-5xl` is gone.
export const handle = { wide: true };

export default function RunObservations() {
  const { id } = useParams();
  const query = useRunObservations(id);
  const [typeFilter, setTypeFilter] = useState<TypeFilter>("painpoint");
  const [statusFilter, setStatusFilter] = useState<StatusFilter>("open");

  const observations = useMemo<RunObservation[]>(
    () => query.data?.observations ?? [],
    [query.data],
  );
  const visible = useMemo(() => {
    return observations.filter((observation) => {
      if (typeFilter !== "all" && observation.category !== typeFilter) return false;
      if (statusFilter !== "all") {
        const triage = leadingLinkState(observation.linked_work_items ?? []);
        if (statusFilter === "open" && triage === "closed") return false;
        if (statusFilter !== "open" && triage !== statusFilter) return false;
      }
      return true;
    });
  }, [observations, typeFilter, statusFilter]);

  if (query.isLoading) {
    return <LoadingState label="Loading observations…" />;
  }
  if (query.error) {
    return (
      <ErrorState
        title="Observations failed to load"
        description={String(query.error)}
      />
    );
  }
  if (!observations.length) {
    return (
      <div className="py-12">
        <EmptyState
          title="No observations"
          description="This run's stages have not recorded any observations."
        />
      </div>
    );
  }
  return (
    <div className="mx-auto max-w-[96rem] space-y-6">
      <div className="flex items-center justify-between">
        <p className="text-xs text-fg-muted">
          {visible.length} of {observations.length} observations
        </p>
        <div className="flex items-center gap-2">
          <select
            aria-label="Type filter"
            value={typeFilter}
            onChange={(event) => setTypeFilter(event.target.value as TypeFilter)}
            className="rounded-md border border-line bg-panel px-2 py-1 text-xs text-fg"
          >
            <option value="all">type: all</option>
            <option value="painpoint">type: painpoint</option>
            <option value="note">type: note</option>
          </select>
          <select
            aria-label="Status filter"
            value={statusFilter}
            onChange={(event) => setStatusFilter(event.target.value as StatusFilter)}
            className="rounded-md border border-line bg-panel px-2 py-1 text-xs text-fg"
          >
            <option value="all">status: all</option>
            <option value="open">status: open</option>
            <option value="untriaged">status: untriaged</option>
            <option value="filed">status: filed</option>
            <option value="in_progress">status: in progress</option>
            <option value="closed">status: closed</option>
          </select>
        </div>
      </div>
      <div className="overflow-hidden rounded-md border border-line bg-panel">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-line bg-panel-alt text-left text-xs font-medium text-fg-3">
              <th className="px-4 py-2.5 font-medium">Stage</th>
              <th className="px-4 py-2.5 font-medium">Type</th>
              <th className="px-4 py-2.5 font-medium">State</th>
              <th className="px-4 py-2.5 font-medium">Data</th>
              <th className="px-4 py-2.5 font-medium">Work item</th>
            </tr>
          </thead>
          <tbody>
            {visible.map((observation) => {
              const triage = leadingLinkState(observation.linked_work_items ?? []);
              return (
                <tr
                  key={observation.seq}
                  className="border-b border-line align-top last:border-b-0"
                >
                  <td className="px-4 py-3 whitespace-nowrap font-mono text-fg-2">
                    {observation.stage ? (
                      <Link
                        to={`/runs/${encodeURIComponent(id ?? "")}/stages/${encodeURIComponent(observation.stage)}`}
                        className="text-teal-500 hover:underline"
                      >
                        {observation.stage}
                      </Link>
                    ) : (
                      EMPTY_VALUE
                    )}
                  </td>
                  <td className="px-4 py-3 whitespace-nowrap text-fg-2">
                    {observation.category}
                  </td>
                  <td className="px-4 py-3 whitespace-nowrap">
                    <span
                      className={`rounded-full border px-1.5 py-0.5 font-mono text-[11px] uppercase tracking-wide ${TRIAGE_BADGE_CLASS[triage]}`}
                    >
                      {TRIAGE_LABEL[triage]}
                    </span>
                  </td>
                  <td className="px-4 py-3 text-fg-2">
                    <span className="whitespace-pre-wrap">{observation.text}</span>
                  </td>
                  <td className="px-4 py-3 whitespace-nowrap font-mono text-fg-2">
                    {(observation.linked_work_items ?? []).length
                      ? (observation.linked_work_items ?? []).map((link) => (
                          <span key={link.item_key} className="mr-2 inline-block">
                            {link.item_url ? (
                              <a
                                href={link.item_url}
                                target="_blank"
                                rel="noreferrer"
                                className="text-teal-500 hover:underline"
                              >
                                {link.item_key}
                              </a>
                            ) : (
                              link.item_key
                            )}
                            {link.item_state && (
                              <span className="ml-1 text-fg-muted">
                                ({ITEM_STATE_LABEL[link.item_state] ?? link.item_state})
                              </span>
                            )}
                          </span>
                        ))
                      : EMPTY_VALUE}
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
