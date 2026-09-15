// Fork-only presence pin (fabro-7627, salvaged from run
// 01M2J1BA2JC4S6SA10YEJHHMVP 2026-09-15): the Resume lifecycle action
// must keep its gating semantics and stay exported from run-actions —
// an upstream merge that drops the fork feature reds this file instead
// of silently regressing the UI (fork-file policy, fabro-8ee1 class).

import { describe, expect, test } from "bun:test";
import type { Run } from "../lib/api-client";
import { canResume, resumeRun } from "./run-actions";

function runWith(overrides: Record<string, unknown>): Run {
  return {
    id: "run_1",
    lifecycle: { status: { kind: "failed", reason: "workflow_error" }, archived: false },
    repository: { repo: "denkhaus/fabro", branch: "denkhaus" },
    ...overrides,
  } as unknown as Run;
}

describe("canResume gating (fork pin, fabro-7627)", () => {
  test("failed git-backed run can resume", () => {
    expect(canResume(runWith({}))).toBe(true);
  });

  test("cancelled failure has nothing to resume", () => {
    expect(
      canResume(runWith({ lifecycle: { status: { kind: "failed", reason: "cancelled" }, archived: false } })),
    ).toBe(false);
  });

  test("dead run can resume, succeeded cannot", () => {
    expect(canResume(runWith({ lifecycle: { status: { kind: "dead" }, archived: false } }))).toBe(true);
    expect(canResume(runWith({ lifecycle: { status: { kind: "succeeded", reason: "completed" }, archived: false } }))).toBe(false);
  });

  test("archived or non-git runs cannot resume", () => {
    expect(canResume(runWith({ lifecycle: { status: { kind: "failed", reason: "workflow_error" }, archived: true } }))).toBe(false);
    expect(canResume(runWith({ repository: null }))).toBe(false);
    expect(canResume(null)).toBe(false);
  });

  test("resumeRun posts to the fork resume endpoint", () => {
    expect(typeof resumeRun).toBe("function");
  });
});
