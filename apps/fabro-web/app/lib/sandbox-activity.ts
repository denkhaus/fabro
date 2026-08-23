import type { EventEnvelope } from "@qltysh/fabro-api-client";

/**
 * Derived sandbox activity from run events.
 *
 * The engine already emits the raw transitions (sandbox.snapshot.creating,
 * sandbox.git.started, setup.started, …) but nothing in the UI turns them
 * into human-readable progress. This module is that pure mapping: given the
 * ordered event stream it derives
 *
 *  - `currentSandboxActivity`: the in-flight activity a viewer should see
 *    while the sandbox initializes (run header), and
 *  - `sandboxActivitySpans`: start/end intervals for the events waterfall.
 *
 * Keep this module free of React, SWR, and SSE: consumers feed it events and
 * render the result.
 */

export type SandboxActivityKind = "building" | "pulling" | "cloning" | "setting-up";

export interface SandboxActivity {
  kind: SandboxActivityKind;
  label: string;
  detail: string | null;
  /** Building a runner image from a Dockerfile is typically a long one-time
   * operation; callers use this to show a "first build" hint. */
  firstBuild: boolean;
  startMs: number;
}

export interface SandboxActivitySpan {
  kind: SandboxActivityKind;
  label: string;
  detail: string | null;
  startMs: number;
  endMs: number | null;
  failed: boolean;
}

interface OpenSpan {
  kind: SandboxActivityKind;
  /** Short span label for dense views (waterfall rows). */
  label: string;
  /** Image name for build/pull activities; drives the verbose header label. */
  detail: string | null;
  startMs: number;
  endMs: number | null;
}

interface Properties {
  name?: unknown;
  url?: unknown;
  error?: unknown;
}

function propertiesOf(event: EventEnvelope): Properties {
  const value = (event as { properties?: unknown }).properties;
  return value != null && typeof value === "object" ? (value as Properties) : {};
}

function nameOf(event: EventEnvelope): string | null {
  const name = propertiesOf(event).name;
  return typeof name === "string" && name.length > 0 ? name : null;
}

function tsOf(event: EventEnvelope): number | null {
  const ms = Date.parse(event.ts);
  return Number.isNaN(ms) ? null : ms;
}

function buildOpen(
  kind: "building" | "pulling",
  event: EventEnvelope,
): OpenSpan | null {
  const ms = tsOf(event);
  if (ms == null) return null;
  // Waterfall rows are narrow: use the short "runner image" wording there
  // and keep the full tag for the run header, which has the space for it.
  const label = kind === "building" ? "Build runner image" : "Pull image";
  return {
    kind,
    label,
    detail: nameOf(event),
    startMs: ms,
    endMs: null,
  };
}

function simpleOpen(
  kind: "cloning" | "setting-up",
  event: EventEnvelope,
): OpenSpan | null {
  const ms = tsOf(event);
  if (ms == null) return null;
  const label = kind === "cloning" ? "Clone repository" : "Setup workspace";
  return { kind, label, detail: null, startMs: ms, endMs: null };
}

function currentLabel(open: OpenSpan): string {
  const image = open.detail;
  switch (open.kind) {
    case "building":
      return image != null
        ? `Building image ${image}`
        : "Building runner image";
    case "pulling":
      return image != null ? `Pulling image ${image}` : "Pulling image";
    case "cloning":
      return "Cloning repository";
    case "setting-up":
      return "Setting up workspace";
  }
}

/**
 * The latest activity that started and has not completed. Returns null when
 * the stream shows no in-flight sandbox work (idle, failed, or finished).
 */
export function currentSandboxActivity(
  events: ReadonlyArray<EventEnvelope> | undefined | null,
): SandboxActivity | null {
  const open = trackOpenSpans(events ?? []).open;
  if (!open) return null;
  return {
    kind: open.kind,
    label: currentLabel(open),
    detail: open.detail,
    firstBuild: open.kind === "building",
    startMs: open.startMs,
  };
}

/**
 * Start/end intervals for every sandbox activity observed in the stream.
 * An activity ends at its completion event; `sandbox.failed` closes any
 * still-open activity and marks it failed.
 */
export function sandboxActivitySpans(
  events: ReadonlyArray<EventEnvelope> | undefined | null,
): SandboxActivitySpan[] {
  return trackOpenSpans(events ?? []).spans;
}

interface TrackResult {
  open: OpenSpan | null;
  spans: SandboxActivitySpan[];
}

function trackOpenSpans(
  events: ReadonlyArray<EventEnvelope>,
): TrackResult {
  let open: OpenSpan | null = null;
  const spans: SandboxActivitySpan[] = [];

  const close = (ms: number, failed: boolean) => {
    if (!open) return;
    spans.push({ ...open, endMs: ms, failed });
    open = null;
  };

  for (const event of events) {
    const ms = tsOf(event);
    if (ms == null) continue;
    switch (event.event) {
      case "sandbox.snapshot.creating":
        close(ms, false);
        open = buildOpen("building", event);
        break;
      case "sandbox.snapshot.pulling":
        close(ms, false);
        open = buildOpen("pulling", event);
        break;
      case "sandbox.git.started":
        close(ms, false);
        open = simpleOpen("cloning", event);
        break;
      case "setup.started":
        close(ms, false);
        open = simpleOpen("setting-up", event);
        break;
      case "sandbox.snapshot.ready":
      case "sandbox.snapshot.failed":
      case "sandbox.git.completed":
      case "sandbox.git.failed":
      case "setup.completed":
        close(ms, event.event.endsWith(".failed"));
        break;
      case "sandbox.failed":
        close(ms, true);
        break;
    }
  }

  return { open, spans: [...spans, ...(open ? [{ ...open, endMs: null, failed: false }] : [])] };
}
