import { describe, expect, test } from "bun:test";
import type { BoardColumn, Run } from "@qltysh/fabro-api-client";
import { MemoryRouter } from "react-router";
import TestRenderer, { act } from "react-test-renderer";

import {
  buildBoardColumns,
  buildFilterOptions,
  loadStoredRunsWorkspaceSearchParams,
  placeArchivedColumnLast,
  persistRunsWorkspacePreferences,
  PrCard,
  RUNS_PREFERENCES_STORAGE_KEY,
  runsQuickStartCommands,
  shouldRefreshBoardForEvent,
} from "./runs";
import { summarizeBatchLifecycleAction } from "../components/runs-list/batch-lifecycle";
import { mapRunListItem, type RunItem } from "../data/runs";
import { TEST_PRINCIPAL, makeUsage } from "../lib/test-fixtures";

function boardRun(id: string, column: BoardColumn, questionText?: string): Run {
  const status =
    column === "blocked"
      ? { kind: "blocked" as const, blocked_reason: "human_input_required" as const }
      : column === "succeeded"
        ? { kind: "succeeded" as const, reason: "completed" }
      : column === "failed"
        ? { kind: "failed" as const, reason: "workflow_error" as const }
        : column === "pending"
          ? { kind: "pending" as const, reason: "approval_required" as const }
          : column === "runnable"
            ? { kind: "runnable" as const }
            : column === "initializing"
              ? { kind: "starting" as const }
              : { kind: "running" as const };
  return {
    id,
    goal:             `Run ${id}`,
    title:            `Run ${id}`,
    workflow:         { slug: "test", name: "Test", graph_name: null, node_count: 0, edge_count: 0 },
    automation:       null,
    repository:       { name: "repo", origin_url: null, provider: "unknown" },
    created_by:       TEST_PRINCIPAL,
    origin:           { kind: "api" },
    labels:           {},
    lifecycle:        {
      status,
      approval: null,
      pending_control: null,
      queue_position:  null,
      error:           null,
      archived:        column === "archived",
      archived_at:     column === "archived" ? "2026-04-19T12:05:00Z" : null,
    },
    sandbox:          null,
    models:           [],
    source_directory: null,
    timestamps:       {
      created_at:     "2026-04-19T12:00:00Z",
      started_at:     null,
      last_event_at:  null,
      completed_at:   null,
    },
    usage:          makeUsage(),
    size:             "XS",
    diff:             null,
    pull_request:     null,
    current_question: questionText ? { text: questionText } : null,
    superseded_by:    null,
    retried_from:     null,
    links:            { web: null },
  };
}

describe("claimed seed lift (fabro-9711/3a5e)", () => {
  test("mapRunListItem carries claimed_seed_id when a run claimed", () => {
    const run = boardRun("claimed-run", "running");
    run.claimed_seed_id = "fabro-9711";
    const item = mapRunListItem(run);
    expect(item.claimedSeedId).toBe("fabro-9711");
  });

  test("a run that never claimed maps no seed id", () => {
    const item = mapRunListItem(boardRun("unclaimed-run", "running"));
    expect(item.claimedSeedId).toBeUndefined();
  });
});

describe("runs route board mapping", () => {
  test("a quota-blocked run carries its block reason into the board item (fabro-2c4a)", () => {
    const quota = boardRun("quota-parked-run", "blocked");
    quota.lifecycle.status = { kind: "blocked", blocked_reason: "quota_rate_limit" };
    const waiting = boardRun("waiting-run", "blocked", "Answer me?");

    const quotaItem = mapRunListItem(quota);
    const waitingItem = mapRunListItem(waiting);

    expect(quotaItem.blockedReason).toBe("quota_rate_limit");
    expect(quotaItem.lifecycleStatus).toBe("blocked");
    expect(waitingItem.blockedReason).toBe("human_input_required");
  });

  test("keeps blocked runs in the blocked lane and preserves question text", () => {
    const columns = buildBoardColumns(
      {
        data: [
          boardRun("paused-run", "running"),
          boardRun("blocked-run", "blocked", "Older unresolved question?"),
        ],
      },
      false,
    );

    expect(columns.find((column) => column.id === "running")?.items.map((item) => item.id)).toContain("paused-run");
    expect(columns.find((column) => column.id === "blocked")?.items.map((item) => item.id)).toContain("blocked-run");
    expect(columns.find((column) => column.id === "blocked")?.items[0]?.question).toBe("Older unresolved question?");
  });

  test("renders an archived column when includeArchived is true", () => {
    const columns = buildBoardColumns(
      {
        data: [
          boardRun("succeeded-run", "succeeded"),
          boardRun("archived-run", "archived"),
        ],
      },
      true,
    );

    expect(columns.map((column) => column.id)).toEqual([
      "pending",
      "runnable",
      "initializing",
      "running",
      "blocked",
      "human_needed",
      "succeeded",
      "failed",
      "archived",
    ]);
    expect(
      columns.find((column) => column.id === "archived")?.items.map((item) => item.id),
    ).toEqual(["archived-run"]);
    expect(
      columns.find((column) => column.id === "succeeded")?.items.map((item) => item.id),
    ).toEqual(["succeeded-run"]);
  });

  test("omits the archived column when includeArchived is false", () => {
    const columns = buildBoardColumns(
      { data: [boardRun("succeeded-run", "succeeded")] },
      false,
    );

    expect(columns.some((column) => column.id === "archived")).toBe(false);
  });

  test("places the archived column last when active", () => {
    const columns = buildBoardColumns(
      {
        data: [
          boardRun("running-run", "running"),
          boardRun("succeeded-run", "succeeded"),
          boardRun("archived-run", "archived"),
        ],
      },
      true,
    );

    expect(placeArchivedColumnLast(columns, true).map((column) => column.id)).toEqual([
      "pending",
      "runnable",
      "initializing",
      "running",
      "blocked",
      "human_needed",
      "succeeded",
      "failed",
      "archived",
    ]);
  });

  test("refreshes for the lifecycle, the blocking question and the answer", () => {
    expect(shouldRefreshBoardForEvent("run.created")).toBe(true);
    expect(shouldRefreshBoardForEvent("run.lifecycle")).toBe(true);
    expect(shouldRefreshBoardForEvent("invocation.cancel.requested")).toBe(true);
    expect(shouldRefreshBoardForEvent("run.archived")).toBe(true);
    expect(shouldRefreshBoardForEvent("run.unarchived")).toBe(true);
    expect(shouldRefreshBoardForEvent("run.title")).toBe(true);
    expect(shouldRefreshBoardForEvent("wait.state.changed")).toBe(true);
    expect(shouldRefreshBoardForEvent("interview.answered")).toBe(true);
    expect(shouldRefreshBoardForEvent("step.progress.recorded")).toBe(false);
  });

  test("includes the configured server argument for GitHub-auth quick starts", () => {
    expect(runsQuickStartCommands(true, "http://127.0.0.1:32276")).toEqual([
      "fabro auth login --server http://127.0.0.1:32276",
      "fabro repo init",
      "fabro run hello",
    ]);
  });

  test("does not show a placeholder server when system info is unavailable", () => {
    expect(runsQuickStartCommands(true)).toEqual([
      "fabro repo init",
      "fabro run hello",
    ]);
  });

  test("summarizes successful batch archive and unarchive actions", () => {
    expect(
      summarizeBatchLifecycleAction("Archive", { requested: 2, succeeded: 2, failed: 0 }),
    ).toEqual({ message: "Archived 2 runs." });
    expect(
      summarizeBatchLifecycleAction("Unarchive", { requested: 1, succeeded: 1, failed: 0 }),
    ).toEqual({ message: "Unarchived 1 run." });
    expect(
      summarizeBatchLifecycleAction("Delete", { requested: 3, succeeded: 3, failed: 0 }),
    ).toEqual({ message: "Deleted 3 runs." });
  });

  test("summarizes partial and failed batch lifecycle actions", () => {
    expect(
      summarizeBatchLifecycleAction("Archive", { requested: 3, succeeded: 2, failed: 1 }),
    ).toEqual({
      message: "Archived 2 of 3 runs. 1 failed.",
      tone:    "error",
    });
    expect(
      summarizeBatchLifecycleAction("Unarchive", { requested: 2, succeeded: 0, failed: 2 }),
    ).toEqual({
      message: "Couldn't unarchive 2 runs. Try again.",
      tone:    "error",
    });
    expect(
      summarizeBatchLifecycleAction("Delete", { requested: 4, succeeded: 3, failed: 1 }),
    ).toEqual({
      message: "Deleted 3 of 4 runs. 1 failed.",
      tone:    "error",
    });
    expect(
      summarizeBatchLifecycleAction("Delete", { requested: 2, succeeded: 0, failed: 2 }),
    ).toEqual({
      message: "Couldn't delete 2 runs. Try again.",
      tone:    "error",
    });
  });
});

describe("runs route filter options", () => {
  function runWith(id: string, repoName: string, workflowName: string): Run {
    const run = boardRun(id, "running");
    return {
      ...run,
      repository: { ...run.repository, name: repoName },
      workflow:   { ...run.workflow, name: workflowName },
    };
  }

  test("derives sorted unique options from run items", () => {
    const items = [
      runWith("a", "qlty/beta", "release"),
      runWith("b", "qlty/alpha", "hello"),
      runWith("c", "qlty/beta", "release"),
    ].map(mapRunListItem);

    expect(buildFilterOptions(items, (item) => item.repo, "all")).toEqual(["alpha", "beta"]);
    expect(buildFilterOptions(items, (item) => item.workflow, "all")).toEqual([
      "hello",
      "release",
    ]);
  });

  test("keeps the active selection when no loaded run matches it", () => {
    const items = [runWith("a", "qlty/beta", "release")].map(mapRunListItem);

    expect(buildFilterOptions(items, (item) => item.repo, "gamma")).toEqual(["beta", "gamma"]);
    expect(buildFilterOptions([], (item) => item.workflow, "release")).toEqual(["release"]);
  });
});

describe("runs route workspace preferences", () => {
  class MemoryStorage {
    values = new Map<string, string>();

    getItem(key: string) {
      return this.values.get(key) ?? null;
    }

    setItem(key: string, value: string) {
      this.values.set(key, value);
    }
  }

  test("missing storage returns default search params", () => {
    expect(loadStoredRunsWorkspaceSearchParams(null).toString()).toBe("");
  });

  test("invalid stored values are ignored", () => {
    const storage = new MemoryStorage();
    storage.setItem(
      RUNS_PREFERENCES_STORAGE_KEY,
      JSON.stringify({
        version:   1,
        view:      "table",
        created:   "tomorrow",
        archived:  "yes",
        sort:      "branch",
        direction: "sideways",
        size:      999,
        hide:      "repo,unknown,elapsed",
        page:      12,
      }),
    );

    expect(loadStoredRunsWorkspaceSearchParams(storage).toString()).toBe("hide=repo%2Celapsed");
  });

  test("valid stored preferences produce canonical URL params", () => {
    const storage = new MemoryStorage();
    storage.setItem(
      RUNS_PREFERENCES_STORAGE_KEY,
      JSON.stringify({
        version:   1,
        view:      "list",
        search:    "retry failures",
        repo:      "qlty/fabro",
        workflow:  "release",
        created:   "7d",
        status:    "running,blocked",
        archived:  true,
        sort:      "updated_at",
        direction: "asc",
        size:      50,
        hide:      "repo,changes",
        page:      4,
      }),
    );

    expect(loadStoredRunsWorkspaceSearchParams(storage).toString()).toBe(
      "view=list&search=retry+failures&repo=qlty%2Ffabro&workflow=release&created=7d&status=running%2Cblocked&archived=1&sort=updated_at&direction=asc&size=50&hide=repo%2Cchanges",
    );
  });

  test("stored archived in a status string migrates into the standalone archived toggle", () => {
    const storage = new MemoryStorage();
    storage.setItem(
      RUNS_PREFERENCES_STORAGE_KEY,
      JSON.stringify({ version: 1, view: "list", status: "running,archived" }),
    );

    const params = loadStoredRunsWorkspaceSearchParams(storage);
    expect(params.get("view")).toBe("list");
    // The "archived" token is stripped out of the status filter and flipped
    // into the separate archived toggle so the two controls don't entangle.
    expect(params.get("status")).toBe("running");
    expect(params.get("archived")).toBe("1");
  });

  test("persisting preferences omits page and stores canonical values", () => {
    const storage = new MemoryStorage();

    persistRunsWorkspacePreferences(
      {
        version:   1,
        view:      "columns",
        search:    "abc",
        repo:      "all",
        workflow:  "all",
        created:   "1d",
        status:    new Set<BoardColumn>(["running", "blocked"]),
        archived:  false,
        sort:      "created_at",
        direction: "asc",
        size:      100,
        hide:      "repo,workflow",
        page:      9,
      },
      storage,
    );

    expect(JSON.parse(storage.getItem(RUNS_PREFERENCES_STORAGE_KEY) ?? "{}")).toEqual({
      version:   1,
      view:      "columns",
      search:    "abc",
      repo:      "all",
      workflow:  "all",
      created:   "1d",
      status:    "running,blocked",
      archived:  false,
      sort:      "created_at",
      direction: "asc",
      size:      100,
      hide:      "repo,workflow",
    });
  });
});

describe("board tile (denkhaus/work#1164)", () => {
  const TILE_RUN_ID = "01M6VB7MHH4H8WGBBTRRSBQKK1"; // a full 26-char ULID

  function tileItem(overrides: Partial<RunItem> = {}): RunItem {
    return {
      id: TILE_RUN_ID,
      repo: "fabro",
      title: "seed #1145: named refusal for Git-target creates",
      workflow: "Develop",
      createdBy: TEST_PRINCIPAL,
      // Constructed from local parts, so the expected dot-date literal
      // below holds in every timezone.
      createdAt: new Date(2026, 11, 20, 18, 33).toISOString(),
      claimedSeedId: "DENKHAUS/WORK#1145",
      ...overrides,
    };
  }

  function renderTile(item: RunItem): string {
    let renderer: TestRenderer.ReactTestRenderer | undefined;
    act(() => {
      renderer = TestRenderer.create(
        <MemoryRouter>
          <PrCard pr={item} />
        </MemoryRouter>,
      );
    });
    const json = JSON.stringify(renderer!.toJSON());
    renderer!.unmount();
    return json;
  }

  test("row 1 shows the workflow beside the repo; rows 3-4 carry full id, dot-date and work item", () => {
    const json = renderTile(tileItem());
    expect(json).toContain("Develop");
    expect(json).toContain(TILE_RUN_ID);
    expect(json).not.toContain("…");
    expect(json).toContain("20.12.2026 18:33");
    expect(json).toContain("DENKHAUS/WORK#1145");
  });

  test("the PR chip colours merged, open and closed distinctly; unknown stays muted", () => {
    const base = {
      number: 506,
      pullRequestUrl: "https://github.com/denkhaus/fabro/pull/506",
    };
    const merged = renderTile(tileItem({ ...base, pullRequestState: "merged" }));
    const open = renderTile(tileItem({ ...base, pullRequestState: "open" }));
    const closed = renderTile(tileItem({ ...base, pullRequestState: "closed" }));
    const unknown = renderTile(tileItem(base));

    expect(merged).toContain("text-mint");
    expect(merged).not.toContain("text-cyan-500");
    expect(open).toContain("text-cyan-500");
    expect(open).not.toContain("text-mint");
    expect(closed).toContain("text-fg-muted");
    expect(closed).not.toContain("text-mint");
    expect(closed).not.toContain("text-cyan-500");
    expect(unknown).toContain("text-fg-muted");
    expect(unknown).not.toContain("text-mint");
    expect(unknown).not.toContain("text-cyan-500");
  });

  test("mapRunListItem reads pull_request.state when the projection ships it, unknown otherwise", () => {
    const run = boardRun("stateful-run", "running");
    run.pull_request = {
      number: 506,
      html_url: "https://github.com/denkhaus/fabro/pull/506",
    } as Run["pull_request"];
    expect(mapRunListItem(run).pullRequestState).toBeUndefined();

    (run.pull_request as { state?: unknown }).state = "merged";
    expect(mapRunListItem(run).pullRequestState).toBe("merged");

    (run.pull_request as { state?: unknown }).state = "garbage";
    expect(mapRunListItem(run).pullRequestState).toBeUndefined();
  });
});
