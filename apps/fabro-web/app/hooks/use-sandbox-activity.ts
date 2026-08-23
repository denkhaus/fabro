import { useEffect, useRef, useState } from "react";
import type { EventEnvelope, PaginatedEventList } from "@qltysh/fabro-api-client";

import { apiNullableData, runInternalsApi } from "../lib/api-client";
import { ListRunEventsOrderEnum } from "@qltysh/fabro-api-client";
import { subscribeToRunEvents } from "../lib/run-events";
import {
  currentSandboxActivity,
  type SandboxActivity,
} from "../lib/sandbox-activity";

/** Newest events kept for derivation; sandbox activity clusters at the
 * start of a run, so a bounded buffer is plenty. */
const EVENT_BUFFER_LIMIT = 200;

/** Descending-tail page size for the seed fetch. */
const SEED_PAGE_LIMIT = 50;

/**
 * Tracks the current sandbox activity (image build/pull, clone, setup) for
 * a run that is currently initializing.
 *
 * Seeding: opening the page mid-build must still show the right activity,
 * so the hook first loads a descending tail of the run's events and derives
 * the state from it. Afterwards the run-scoped SSE stream keeps it live by
 * appending to the same buffer.
 *
 * The hook stays mounted-but-idle outside the `starting` lifecycle status:
 * no tail fetch and no subscription run once `enabled` is false.
 */
export function useSandboxActivity(
  runId: string | undefined,
  enabled: boolean,
): SandboxActivity | null {
  const [activity, setActivity] = useState<SandboxActivity | null>(null);
  const activityRef = useRef<SandboxActivity | null>(null);
  activityRef.current = activity;

  useEffect(() => {
    if (!runId || !enabled) {
      setActivity(null);
      return;
    }

    let cancelled = false;
    const buffer: EventEnvelope[] = [];

    const recompute = () => {
      const next = currentSandboxActivity(buffer);
      const previous = activityRef.current;
      if (
        next?.kind !== previous?.kind ||
        next?.detail !== previous?.detail ||
        next?.startMs !== previous?.startMs
      ) {
        setActivity(next);
      }
    };

    // Seed from a descending tail: sandbox activity events cluster at the
    // start of the stream, so the newest page contains everything needed.
    apiNullableData<PaginatedEventList>(() =>
      runInternalsApi.listRunEvents(
        runId,
        undefined,
        SEED_PAGE_LIMIT,
        undefined,
        ListRunEventsOrderEnum.DESC,
      ),
    )
      .then((page) => {
        if (cancelled || !page) return;
        buffer.push(...[...page.data].reverse());
        recompute();
      })
      .catch(() => {
        // The SSE stream still drives updates; a failed seed just means a
        // blank label until the next event arrives.
      });

    const unsubscribe = subscribeToRunEvents(runId, () => undefined, undefined, {
      debounceMs: 0,
      onEvent: (payload) => {
        if (cancelled || payload.run_id !== runId) return;
        const envelope = payloadToEnvelope(payload);
        if (!envelope) return;
        buffer.push(envelope);
        if (buffer.length > EVENT_BUFFER_LIMIT) {
          buffer.splice(0, buffer.length - EVENT_BUFFER_LIMIT);
        }
        recompute();
      },
    });

    return () => {
      cancelled = true;
      unsubscribe();
    };
  }, [runId, enabled]);

  return activity;
}

function payloadToEnvelope(
  payload: {
    id?: string;
    seq?: number;
    ts?: string;
    run_id?: string;
    event?: string;
    properties?: Record<string, unknown>;
  },
): EventEnvelope | null {
  if (!payload.event || !payload.ts) return null;
  return {
    id: payload.id ?? `sse-${payload.seq ?? payload.ts}`,
    seq: payload.seq ?? 0,
    ts: payload.ts,
    run_id: payload.run_id ?? "",
    event: payload.event,
    properties: payload.properties,
  } as EventEnvelope;
}
