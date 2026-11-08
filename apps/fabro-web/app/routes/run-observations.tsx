import { useParams } from "react-router";
import type { RunObservation } from "@qltysh/fabro-api-client";

import { EmptyState, ErrorState, LoadingState } from "../components/state";
import { useRunObservations } from "../lib/queries";

/**
 * The additive fields the observations read carries that the generated
 * client does not know yet (the client regen is a local step; the
 * server's response already includes them — spec: `RunObservation`).
 */
export interface ObservationWorkItemLink {
  item_key: string;
  linked_at?: string | null;
}

export type RunObservationWithStage = RunObservation & {
  stage?: string | null;
  linked_work_items?: ObservationWorkItemLink[];
};

const EMPTY_VALUE = "—";

export const handle = { wide: true };

export default function RunObservations() {
  const { id } = useParams();
  const query = useRunObservations(id);

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
  const observations = (query.data?.observations ?? []) as RunObservationWithStage[];
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
    <div className="mx-auto max-w-5xl space-y-6">
      <div className="overflow-hidden rounded-md border border-line">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-line bg-panel/60 text-left text-xs font-medium text-fg-3">
              <th className="px-4 py-2.5 font-medium">Stage</th>
              <th className="px-4 py-2.5 font-medium">Type</th>
              <th className="px-4 py-2.5 font-medium">Data</th>
              <th className="px-4 py-2.5 font-medium">Work item</th>
            </tr>
          </thead>
          <tbody>
            {observations.map((observation) => (
              <tr
                key={observation.seq}
                className="border-b border-line align-top last:border-b-0"
              >
                <td className="px-4 py-3 whitespace-nowrap font-mono text-fg-2">
                  {observation.stage ?? EMPTY_VALUE}
                </td>
                <td className="px-4 py-3 whitespace-nowrap text-fg-2">
                  {observation.category}
                </td>
                <td className="px-4 py-3 text-fg-2">
                  <span className="whitespace-pre-wrap">{observation.text}</span>
                </td>
                <td className="px-4 py-3 whitespace-nowrap font-mono text-fg-2">
                  {(observation.linked_work_items ?? []).length
                    ? observation
                        .linked_work_items!.map((link) => link.item_key)
                        .join(", ")
                    : EMPTY_VALUE}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
