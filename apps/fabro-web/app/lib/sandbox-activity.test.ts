import { describe, expect, test } from "bun:test";
import type { EventEnvelope } from "@qltysh/fabro-api-client";

import {
  currentSandboxActivity,
  sandboxActivitySpans,
} from "./sandbox-activity";

const T_CREATING = "2026-05-23T12:00:04.500Z";
const T_READY = "2026-05-23T12:01:34.000Z";
const T_PULLING = "2026-05-23T12:00:04.500Z";
const T_PULLED = "2026-05-23T12:00:07.000Z";
const T_GIT_START = "2026-05-23T12:01:35.000Z";
const T_GIT_DONE = "2026-05-23T12:01:41.000Z";
const T_SETUP_START = "2026-05-23T12:01:42.000Z";
const T_SETUP_DONE = "2026-05-23T12:01:43.000Z";

function makeEvent(
  name: string,
  ts: string,
  seq: number,
  properties?: Record<string, unknown>,
): EventEnvelope {
  return {
    id: `evt-${seq}`,
    seq,
    ts,
    run_id: "run-1",
    event: name,
    ...(properties ? { properties } : {}),
  } as EventEnvelope;
}

describe("currentSandboxActivity", () => {
  test("null for empty events", () => {
    expect(currentSandboxActivity([])).toBe(null);
  });

  test("building image while snapshot.creating is in flight", () => {
    const activity = currentSandboxActivity([
      makeEvent("sandbox.snapshot.creating", T_CREATING, 1, {
        name: "fabro-runner-03237792691a",
      }),
    ]);
    expect(activity).toEqual({
      kind: "building",
      label: "Building image fabro-runner-03237792691a",
      detail: "fabro-runner-03237792691a",
      firstBuild: true,
      startMs: Date.parse(T_CREATING),
    });
  });

  test("building without an image name falls back to the short label", () => {
    const activity = currentSandboxActivity([
      makeEvent("sandbox.snapshot.creating", T_CREATING, 1),
    ]);
    expect(activity?.label).toBe("Building runner image");
    expect(activity?.detail).toBe(null);
  });

  test("pulling image while snapshot.pulling is in flight", () => {
    const activity = currentSandboxActivity([
      makeEvent("sandbox.snapshot.pulling", T_PULLING, 1, {
        name: "buildpack-deps:noble",
      }),
    ]);
    expect(activity).toEqual({
      kind: "pulling",
      label: "Pulling image buildpack-deps:noble",
      detail: "buildpack-deps:noble",
      firstBuild: false,
      startMs: Date.parse(T_PULLING),
    });
  });

  test("cloning repository after git.started", () => {
    const activity = currentSandboxActivity([
      makeEvent("sandbox.snapshot.creating", T_CREATING, 1, {
        name: "fabro-runner-03237792691a",
      }),
      makeEvent("sandbox.snapshot.ready", T_READY, 2, {
        name: "fabro-runner-03237792691a",
        duration_ms: 89_431,
      }),
      makeEvent("sandbox.git.started", T_GIT_START, 3, {
        url: "https://github.com/org/repo",
        branch: "main",
      }),
    ]);
    expect(activity?.kind).toBe("cloning");
    expect(activity?.label).toBe("Cloning repository");
    expect(activity?.startMs).toBe(Date.parse(T_GIT_START));
  });

  test("setting up workspace after setup.started", () => {
    const activity = currentSandboxActivity([
      makeEvent("setup.started", T_SETUP_START, 1),
    ]);
    expect(activity?.kind).toBe("setting-up");
    expect(activity?.label).toBe("Setting up workspace");
  });

  test("null once the latest activity completed", () => {
    const activity = currentSandboxActivity([
      makeEvent("sandbox.snapshot.pulling", T_PULLING, 1, {
        name: "buildpack-deps:noble",
      }),
      makeEvent("sandbox.snapshot.ready", T_PULLED, 2, {
        name: "buildpack-deps:noble",
      }),
      makeEvent("sandbox.git.completed", T_GIT_DONE, 3),
    ]);
    expect(activity).toBe(null);
  });

  test("failed build is not reported as current activity", () => {
    const activity = currentSandboxActivity([
      makeEvent("sandbox.snapshot.creating", T_CREATING, 1, {
        name: "fabro-runner-x",
      }),
      makeEvent("sandbox.snapshot.failed", T_READY, 2, {
        name: "fabro-runner-x",
        error: "boom",
      }),
    ]);
    expect(activity).toBe(null);
  });
});

describe("sandboxActivitySpans", () => {
  test("empty for no sandbox events", () => {
    expect(sandboxActivitySpans([])).toEqual([]);
  });

  test("build span closes at snapshot.ready with duration", () => {
    const spans = sandboxActivitySpans([
      makeEvent("sandbox.snapshot.creating", T_CREATING, 1, {
        name: "fabro-runner-03237792691a",
      }),
      makeEvent("sandbox.snapshot.ready", T_READY, 2, {
        name: "fabro-runner-03237792691a",
        duration_ms: 89_431,
      }),
    ]);
    expect(spans).toEqual([
      {
        kind: "building",
        label: "Build runner image",
        detail: "fabro-runner-03237792691a",
        startMs: Date.parse(T_CREATING),
        endMs: Date.parse(T_READY),
        failed: false,
      },
    ]);
  });

  test("in-flight build span has null end", () => {
    const spans = sandboxActivitySpans([
      makeEvent("sandbox.snapshot.creating", T_CREATING, 1, {
        name: "fabro-runner-03237792691a",
      }),
    ]);
    expect(spans).toEqual([
      {
        kind: "building",
        label: "Build runner image",
        detail: "fabro-runner-03237792691a",
        startMs: Date.parse(T_CREATING),
        endMs: null,
        failed: false,
      },
    ]);
  });

  test("failed build span is marked", () => {
    const spans = sandboxActivitySpans([
      makeEvent("sandbox.snapshot.creating", T_CREATING, 1, { name: "img" }),
      makeEvent("sandbox.snapshot.failed", T_READY, 2, {
        name: "img",
        error: "boom",
      }),
    ]);
    expect(spans[0]?.label).toBe("Build runner image");
    expect(spans[0]?.failed).toBe(true);
    expect(spans[0]?.endMs).toBe(Date.parse(T_READY));
  });

  test("clone and setup spans derive from their lifecycle events", () => {
    const spans = sandboxActivitySpans([
      makeEvent("sandbox.git.started", T_GIT_START, 1),
      makeEvent("sandbox.git.completed", T_GIT_DONE, 2),
      makeEvent("setup.started", T_SETUP_START, 3),
      makeEvent("setup.completed", T_SETUP_DONE, 4),
    ]);
    expect(spans.map((s) => s.kind)).toEqual(["cloning", "setting-up"]);
    expect(spans[0]?.endMs).toBe(Date.parse(T_GIT_DONE));
    expect(spans[1]?.endMs).toBe(Date.parse(T_SETUP_DONE));
  });

  test("late sandbox.failed closes any open span as failed", () => {
    const spans = sandboxActivitySpans([
      makeEvent("sandbox.git.started", T_GIT_START, 1),
      makeEvent("sandbox.failed", T_GIT_DONE, 2, { provider: "docker" }),
    ]);
    expect(spans).toEqual([
      {
        kind: "cloning",
        label: "Clone repository",
        detail: null,
        startMs: Date.parse(T_GIT_START),
        endMs: Date.parse(T_GIT_DONE),
        failed: true,
      },
    ]);
  });
});
