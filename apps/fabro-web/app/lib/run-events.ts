import { useEffect } from "react";
import type { RunStreamItem } from "@qltysh/fabro-api-client";
import { useSWRConfig, type Key } from "swr";

import {
  subscribeToCrossTabSse,
  type CrossTabSseCoordinator,
} from "./cross-tab-sse";
import {
  isStreamItemPayload,
  isTerminalLifecycleItem,
  petriEventName,
  petriParsed,
  petriStageLabel,
  platformRecordKind,
} from "./petri-stream";
import { queryKeys } from "./query-keys";
import { getString } from "./unknown";
import {
  createBrowserEventSource,
  subscribeToSharedEventSource,
  type EventPayload,
  type EventSourceLike,
  type MutateFn,
  type SharedEventSubscription,
} from "./sse";

export interface RunEventPayload extends EventPayload {
  id?: string;
  seq?: number;
  event?: string;
  run_id?: string;
  node_id?: string;
  stage_id?: string;
  properties?: Record<string, unknown>;
  /** Set on a Petri run stream item, which is invalidated by its own rules. */
  stream_seq?: number;
  kind?: string;
  item?: unknown;
}

interface RunEventOptions {
  debounceMs?: number;
  coordinator?: CrossTabSseCoordinator;
  onEvent?: (payload: RunEventPayload) => void;
}

const subscriptions = new Map<string, SharedEventSubscription>();

const TERMINAL_EVENTS = new Set(["run.completed", "run.failed"]);
const RUN_SUMMARY_EVENTS = new Set([
  "run.submitted",
  "run.start_requested",
  "run.pending",
  "run.approved",
  "run.denied",
  "run.runnable",
  "run.starting",
  "run.running",
  "run.paused",
  "run.unpaused",
  "run.blocked",
  "run.unblocked",
  "run.cancel.requested",
  "run.pause.requested",
  "run.unpause.requested",
  "run.archived",
  "run.unarchived",
  "run.title.updated",
  "pull_request.created",
  "pull_request.linked",
  "pull_request.unlinked",
]);
const STAGE_EVENTS = new Set([
  "stage.started",
  "stage.completed",
  "stage.failed",
  "stage.retrying",
]);
// Single source of truth: every event type the `eventsToActivity` reducer in
// `routes/run-stages.tsx` consumes. When any of these arrive for a stage we
// currently view, the stage-events SWR key for that stage must be invalidated
// so the panel refetches. The reducer imports this list so the switch stays
// in sync with the invalidation set; if the reducer grows a new case, this
// list is the single edit point.
//
// The lifecycle `STAGE_EVENTS` set is kept separate because it also fans out
// to run-scoped invalidations (stages list, graph, detail).
export const STAGE_ACTIVITY_EVENT_TYPES = [
  "stage.prompt",
  "prompt.completed",
  "agent.message",
  "agent.tool.started",
  "agent.tool.completed",
  "agent.steering.injected",
  "agent.interrupt.injected",
  "agent.round.interrupted",
  "agent.pair.user_message",
  "agent.pair.system_message",
  "command.started",
  "command.completed",
] as const;
export type StageActivityEventType = (typeof STAGE_ACTIVITY_EVENT_TYPES)[number];
const STAGE_ACTIVITY_EVENTS = new Set<string>(STAGE_ACTIVITY_EVENT_TYPES);
// Parallel branches bypass the engine's `stage.started` / `stage.completed`
// lifecycle (the parallel handler dispatches each branch directly), so
// `STAGE_EVENTS` never fires for them. Without this set the stages list never
// refetches while a fork runs and branch rows stay frozen at their first
// observed state.
const PARALLEL_EVENTS = new Set([
  "parallel.started",
  "parallel.branch.started",
  "parallel.branch.completed",
  "parallel.completed",
]);
const INTERVIEW_EVENTS = new Set([
  "interview.started",
  "interview.completed",
  "interview.timeout",
  "interview.interrupted",
]);
const STEERING_EVENTS = new Set([
  "run.interrupt",
  "run.steer",
  "agent.steering.injected",
  "agent.interrupt.injected",
  "agent.round.interrupted",
  "agent.session.activated",
  "agent.session.deactivated",
  "agent.steer.buffered",
  "agent.steer.dropped",
]);
const AGENT_CONTROL_STATE_EVENTS = new Set([
  "agent.round.interrupted",
  "agent.steering.injected",
  "agent.session.deactivated",
]);
const INFERENCE_EVENTS = new Set([
  "agent.llm.started",
  "agent.llm.first_output",
  "agent.llm.retry",
  "agent.message",
  "agent.error",
  "agent.session.ended",
]);
const INFERENCE_TIMING_EVENTS = new Set([
  "agent.llm.started",
  "agent.message",
  "agent.error",
  "agent.session.ended",
]);
const TOOL_TIMING_EVENTS = new Set([
  "agent.tool.started",
  "agent.tool.completed",
]);
const ACP_TIMING_EVENTS = new Set([
  "agent.acp.started",
  "agent.acp.completed",
  "agent.acp.cancelled",
  "agent.acp.timed_out",
]);
// Todo / task mutation events refresh `getRunState` consumers (so per-stage
// todo projections update live) and the run events list.
const TODO_EVENTS = new Set([
  "todo.created",
  "todo.updated",
  "todo.deleted",
]);
// The coding agent's own events whose facts the stage sidebar reads from
// `StageProjection.agent`: route moves, MCP server outcomes, skills,
// subagent lifecycle, and compactions. Each refreshes the run state the
// sidebar renders from, like a todo mutation does. Files the tree wrote
// arrive with `agent.tool.completed`, which already refreshes it.
const AGENT_SESSION_EVENTS = new Set([
  "agent.route.failover",
  "agent.route.failover.stopped",
  "agent.mcp.server.ready",
  "agent.mcp.server.failed",
  "agent.mcp.server.disconnected",
  "agent.skills.discovered",
  "agent.skill.activated",
  "agent.sub.spawned",
  "agent.sub.turn.started",
  "agent.sub.completed",
  "agent.sub.failed",
  "agent.sub.closed",
  "agent.compaction.completed",
]);

function liveTimingKeys(runId: string): Key[] {
  return [
    queryKeys.runs.detail(runId),
    queryKeys.runs.state(runId),
    queryKeys.runs.usage(runId),
  ];
}

export function queryKeysForRunEvent(
  runId: string,
  event: string,
  stageId?: string,
): Key[] {
  if (event === "checkpoint.completed") {
    return [
      ...queryKeys.runs.filesAllScopes(runId),
      queryKeys.runs.commits(runId),
    ];
  }

  if (TERMINAL_EVENTS.has(event)) {
    return [
      queryKeys.runs.detail(runId),
      queryKeys.runs.state(runId),
      ...queryKeys.runs.filesAllScopes(runId),
      queryKeys.runs.commits(runId),
      queryKeys.runs.usage(runId),
      queryKeys.runs.stages(runId),
      queryKeys.runs.graph(runId, "LR"),
      queryKeys.runs.graph(runId, "TB"),
    ];
  }

  if (RUN_SUMMARY_EVENTS.has(event)) {
    return [queryKeys.runs.detail(runId)];
  }

  if (INTERVIEW_EVENTS.has(event)) {
    return [
      queryKeys.runs.questions(runId, 25, 0),
      queryKeys.runs.detail(runId),
    ];
  }

  if (STAGE_EVENTS.has(event)) {
    const keys: Key[] = [
      queryKeys.runs.stages(runId),
      queryKeys.runs.usage(runId),
      queryKeys.runs.events(runId, 1000),
      queryKeys.runs.graph(runId, "LR"),
      queryKeys.runs.graph(runId, "TB"),
      queryKeys.runs.detail(runId),
      queryKeys.runs.state(runId),
    ];
    if (stageId) {
      keys.push(queryKeys.runs.stageEvents(runId, stageId));
      keys.push(queryKeys.runs.stageContextWindow(runId, stageId));
    }
    return keys;
  }

  if (PARALLEL_EVENTS.has(event)) {
    const keys: Key[] = [
      queryKeys.runs.stages(runId),
      queryKeys.runs.events(runId, 1000),
      queryKeys.runs.graph(runId, "LR"),
      queryKeys.runs.graph(runId, "TB"),
    ];
    if (stageId) {
      keys.push(queryKeys.runs.stageEvents(runId, stageId));
    }
    return keys;
  }

  if (STEERING_EVENTS.has(event)) {
    const keys: Key[] = [queryKeys.runs.events(runId, 1000)];
    if (AGENT_CONTROL_STATE_EVENTS.has(event)) {
      keys.unshift(queryKeys.runs.state(runId));
    }
    if (event === "agent.round.interrupted") {
      keys.unshift(
        queryKeys.runs.detail(runId),
        queryKeys.runs.usage(runId),
      );
    }
    if (stageId) {
      keys.push(queryKeys.runs.stageEvents(runId, stageId));
      keys.push(queryKeys.runs.stageContextWindow(runId, stageId));
    }
    return keys;
  }

  if (INFERENCE_EVENTS.has(event)) {
    const keys = INFERENCE_TIMING_EVENTS.has(event)
      ? liveTimingKeys(runId)
      : [queryKeys.runs.state(runId)];
    if (stageId) {
      keys.push(queryKeys.runs.stageEvents(runId, stageId));
      if (event === "agent.message") {
        keys.push(queryKeys.runs.stageContextWindow(runId, stageId));
      }
    }
    return keys;
  }

  if (TOOL_TIMING_EVENTS.has(event)) {
    const keys = liveTimingKeys(runId);
    if (stageId) {
      keys.push(queryKeys.runs.stageEvents(runId, stageId));
      keys.push(queryKeys.runs.stageContextWindow(runId, stageId));
    }
    return keys;
  }

  if (ACP_TIMING_EVENTS.has(event)) {
    const keys = liveTimingKeys(runId);
    if (stageId) {
      keys.push(queryKeys.runs.stageEvents(runId, stageId));
    }
    return keys;
  }

  if (event === "watchdog.timeout") {
    return stageId ? [queryKeys.runs.stageEvents(runId, stageId)] : [];
  }

  if (STAGE_ACTIVITY_EVENTS.has(event)) {
    return stageId
      ? [
          queryKeys.runs.stageEvents(runId, stageId),
          queryKeys.runs.stageContextWindow(runId, stageId),
        ]
      : [];
  }

  if (TODO_EVENTS.has(event) || AGENT_SESSION_EVENTS.has(event)) {
    const keys: Key[] = [
      queryKeys.runs.state(runId),
      queryKeys.runs.events(runId, 1000),
    ];
    if (stageId) {
      keys.push(queryKeys.runs.stageEvents(runId, stageId));
    }
    return keys;
  }

  return [];
}

/**
 * The SWR keys a Petri run stream item invalidates. Petri's events are
 * named `<subject>.<verb>`; a platform record by its `kind`. The stage
 * keys use the subject's `node@visit` label, which is the stage id the
 * projection keys stages by.
 */
export function queryKeysForStreamItem(
  runId: string,
  item: RunStreamItem,
): { keys: Key[]; immediate: boolean } {
  const stageId = petriStageLabel(item);
  const stageKeys: Key[] = stageId
    ? [queryKeys.runs.stageEvents(runId, stageId), queryKeys.runs.stageContextWindow(runId, stageId)]
    : [];
  const stream = queryKeys.runs.stream(runId);

  if (item.kind === "platform") {
    const kind = platformRecordKind(item);
    if (isTerminalLifecycleItem(item)) {
      return { keys: terminalKeys(runId, stream), immediate: true };
    }
    switch (kind) {
      case "checkpoint":
        return {
          keys: [
            ...queryKeys.runs.filesAllScopes(runId),
            queryKeys.runs.commits(runId),
            queryKeys.runs.state(runId),
            stream,
          ],
          immediate: false,
        };
      case "interview.answered":
        return {
          keys: [queryKeys.runs.questions(runId, 25, 0), queryKeys.runs.detail(runId), stream],
          immediate: false,
        };
      default:
        return { keys: [queryKeys.runs.detail(runId), queryKeys.runs.state(runId), stream], immediate: false };
    }
  }

  const name = petriEventName(item);
  switch (name) {
    case "run.finished":
      return { keys: terminalKeys(runId, stream), immediate: false };
    case "run.started":
    case "run.paused":
    case "run.unpaused":
    case "invocation.finished":
    case "invocation.cancel.requested":
    case "run.stalled":
      return { keys: [queryKeys.runs.detail(runId), queryKeys.runs.state(runId), stream], immediate: false };
    case "visit.started":
    case "visit.completed":
    case "retry.scheduled":
    case "wait.state.changed":
    case "admission.decided":
      return {
        keys: [
          queryKeys.runs.stages(runId),
          queryKeys.runs.state(runId),
          queryKeys.runs.detail(runId),
          stream,
          queryKeys.runs.graph(runId, "LR"),
          queryKeys.runs.graph(runId, "TB"),
          ...stageKeys,
        ],
        immediate: false,
      };
    case "step.progress.recorded": {
      const parsed = getString(petriParsed(item), "kind");
      if (parsed === "question" || parsed === "question_expired") {
        return {
          keys: [
            queryKeys.runs.questions(runId, 25, 0),
            queryKeys.runs.detail(runId),
            queryKeys.runs.state(runId),
            stream,
            ...stageKeys,
          ],
          immediate: false,
        };
      }
      return { keys: [queryKeys.runs.state(runId), stream, ...stageKeys], immediate: false };
    }
    case "control.requested":
      return {
        keys: [
          queryKeys.runs.questions(runId, 25, 0),
          queryKeys.runs.detail(runId),
          queryKeys.runs.state(runId),
          stream,
          ...stageKeys,
        ],
        immediate: false,
      };
    case "step.finished":
      return {
        keys: [
          queryKeys.runs.state(runId),
          queryKeys.runs.usage(runId),
          queryKeys.runs.stages(runId),
          queryKeys.runs.detail(runId),
          stream,
          ...stageKeys,
        ],
        immediate: false,
      };
    case "fork.started":
    case "branch.completed":
    case "fork.completed":
    case "node.expanded":
      return {
        keys: [
          queryKeys.runs.stages(runId),
          queryKeys.runs.state(runId),
          stream,
          queryKeys.runs.graph(runId, "LR"),
          queryKeys.runs.graph(runId, "TB"),
        ],
        immediate: false,
      };
    case "routing.resolved":
    case "route.applied":
      return { keys: [stream, ...stageKeys], immediate: false };
    default:
      return { keys: [stream], immediate: false };
  }
}

function terminalKeys(runId: string, stream: Key): Key[] {
  return [
    queryKeys.runs.detail(runId),
    queryKeys.runs.state(runId),
    ...queryKeys.runs.filesAllScopes(runId),
    queryKeys.runs.commits(runId),
    queryKeys.runs.usage(runId),
    queryKeys.runs.stages(runId),
    stream,
    queryKeys.runs.graph(runId, "LR"),
    queryKeys.runs.graph(runId, "TB"),
  ];
}

export function subscribeToRunEvents(
  runId: string,
  mutate: MutateFn,
  eventSourceFactory: (url: string) => EventSourceLike = createBrowserEventSource,
  { debounceMs = 300, coordinator, onEvent }: RunEventOptions = {},
): () => void {
  return subscribeToCrossTabSse<RunEventPayload>({
    coordinator,
    subscriptionKey: `run:${runId}`,
    mutate,
    debounceMs,
    resyncKeys: () => resyncKeysForRun(runId),
    resolveInvalidation: (payload) => {
      if (payload.run_id !== runId) return { keys: [] };
      onEvent?.(payload);
      return runInvalidation(runId, payload);
    },
    fallbackSubscribe: () =>
      subscribeToSharedEventSource<RunEventPayload>({
        subscriptions,
        subscriptionKey: runId,
        url: queryKeys.runs.attachUrl(runId),
        mutate,
        eventSourceFactory,
        debounceMs,
        resolveInvalidation: (payload) => {
          onEvent?.(payload);
          const result = runInvalidation(runId, payload);
          return { ...result, close: result.immediate };
        },
      }),
  });
}

function runInvalidation(runId: string, payload: RunEventPayload) {
  if (isStreamItemPayload(payload)) {
    return queryKeysForStreamItem(runId, payload);
  }
  const event = payload.event;
  if (!event) return { keys: [], immediate: false };

  const stageId = stageIdFromPayload(payload);
  const keys = queryKeysForRunEvent(runId, event, stageId);
  const terminal = TERMINAL_EVENTS.has(event);
  return { keys, immediate: terminal };
}

function resyncKeysForRun(runId: string) {
  return [
    queryKeys.runs.detail(runId),
    queryKeys.runs.state(runId),
    ...queryKeys.runs.filesAllScopes(runId),
    queryKeys.runs.commits(runId),
    queryKeys.runs.usage(runId),
    queryKeys.runs.stages(runId),
    queryKeys.runs.events(runId, 1000),
    queryKeys.runs.stream(runId),
    queryKeys.runs.graph(runId, "LR"),
    queryKeys.runs.graph(runId, "TB"),
    queryKeys.runs.questions(runId, 25, 0),
  ];
}

function stageIdFromPayload(payload: RunEventPayload): string | undefined {
  if (typeof payload.stage_id === "string") return payload.stage_id;
  if (typeof payload.node_id === "string") return payload.node_id;
  const nodeId = payload.properties?.node_id;
  return typeof nodeId === "string" ? nodeId : undefined;
}

/**
 * Synchronizes React/SWR with a run-scoped SSE stream. Changing `runId`
 * resubscribes, and the active subscription is closed on unmount.
 */
export function useRunEvents(runId: string | undefined) {
  const { mutate } = useSWRConfig();

  useEffect(() => {
    if (!runId) return;
    return subscribeToRunEvents(runId, mutate as MutateFn);
  }, [mutate, runId]);
}
