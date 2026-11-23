import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";
import type { BoardColumn, Run } from "@qltysh/fabro-api-client";
import { createElement } from "react";
import { MemoryRouter } from "react-router";
import TestRenderer, { act } from "react-test-renderer";
import { toast as sonnerToast, useSonner } from "sonner";

import {
  buildBoardColumns,
  buildFilterOptions,
  loadStoredRunsWorkspaceSearchParams,
  placeArchivedColumnLast,
  persistRunsWorkspacePreferences,
  PrCard,
  resolveBoardDrop,
  RUNS_PREFERENCES_STORAGE_KEY,
  runsQuickStartCommands,
  shouldRefreshBoardForEvent,
} from "./runs";
import { summarizeBatchLifecycleAction } from "../components/runs-list/batch-lifecycle";
import { mapRunListItem, type RunItem } from "../data/runs";
import { TEST_PRINCIPAL, makeUsage } from "../lib/test-fixtures";
import { setupReactTestEnv, textContent } from "../lib/test-utils";

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
            : column === "human_needed"
              ? { kind: "human_needed" as const }
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

  test("an archived needs-human park stays in the Needs human column (work#1508)", () => {
    // The fork.43 incident: the operator looked under Needs human and
    // found nothing while `fabro ps -a` showed the park plainly.
    const archivedPark = boardRun("archived-park", "human_needed");
    archivedPark.lifecycle.archived = true;
    archivedPark.lifecycle.archived_at = "2026-11-19T00:52:27Z";

    // Without the archived column (the board's default), the park must
    // still surface under Needs human; the plain archived run drops.
    const columns = buildBoardColumns(
      { data: [archivedPark, boardRun("archived-run", "archived")] },
      false,
    );
    expect(
      columns.find((column) => column.id === "human_needed")?.items.map((item) => item.id),
    ).toEqual(["archived-park"]);
    expect(columns.some((column) => column.id === "archived")).toBe(false);

    // With the archived column open, the park stays a Needs-human
    // citizen — only the plain archived run lands there.
    const withArchived = buildBoardColumns(
      { data: [archivedPark, boardRun("archived-run", "archived")] },
      true,
    );
    expect(
      withArchived.find((column) => column.id === "human_needed")?.items.map((item) => item.id),
    ).toEqual(["archived-park"]);
    expect(
      withArchived.find((column) => column.id === "archived")?.items.map((item) => item.id),
    ).toEqual(["archived-run"]);
  });

  test("an archived park keeps its archived lifecycle mapping (work#1508)", () => {
    // The overlay stays on the item: the unarchive affordance keys on
    // lifecycleStatus === "archived", and the card's label stays honest
    // ("Archived") while the column buckets it under Needs human.
    const archivedPark = boardRun("archived-park", "human_needed");
    archivedPark.lifecycle.archived = true;
    archivedPark.lifecycle.archived_at = "2026-11-19T00:52:27Z";

    const item = mapRunListItem(archivedPark);
    expect(item.lifecycleStatus).toBe("archived");
    expect(item.lifecycleStatusLabel).toBe("Archived");
    expect(item.column).toBe("human_needed");
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

  test("row 1 shows the workflow beside the repo; rows 3-4 carry full id and dot-date with relative age", () => {
    const json = renderTile(tileItem());
    expect(json).toContain("Develop");
    expect(json).toContain(TILE_RUN_ID);
    expect(json).not.toContain("…");
    // The time label pairs the absolute dot-date with the relative age
    // on the creation clock (denkhaus/work#1203); the fixture date is
    // in the future, so the clamped relative arm reads "just now".
    expect(json).toContain("20.12.2026 18:33");
    expect(json).toContain(String.raw`" - "`);
    expect(json).toContain("just now");
    expect(json).toContain("DENKHAUS/WORK#1145");
  });

  test("a fresh tile brightens the time element; an old tile reads relative days", () => {
    const fresh = renderTile(tileItem({ createdAt: new Date(Date.now() - 5 * 60_000).toISOString() }));
    expect(fresh).toContain("5 minutes ago");
    // The fresh time element carries the brighter step.
    expect(fresh).toContain("text-fg-3");
    const old = renderTile(tileItem({ createdAt: new Date(Date.now() - 3 * 86_400_000).toISOString() }));
    expect(old).toContain("3 days ago");
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

describe("board owner-park drop resolution (work#1345)", () => {
  function ownerParkedRun(id: string): Run {
    const run = boardRun(id, "blocked");
    run.lifecycle.status = { kind: "blocked", blocked_reason: "owner_pending" };
    return run;
  }

  function columnsFor(runs: Run[]) {
    return buildBoardColumns({ data: runs }, false);
  }

  test("a failed run dropped on the blocked column parks", () => {
    const columns = columnsFor([boardRun("failed-run", "failed")]);
    expect(resolveBoardDrop(columns, "failed-run", "column:blocked")).toEqual({
      kind: "park",
      runId: "failed-run",
    });
  });

  test("a failed run dropped on a card inside the blocked column parks", () => {
    const columns = columnsFor([
      boardRun("failed-run", "failed"),
      boardRun("waiting-run", "blocked", "Older unresolved question?"),
    ]);
    expect(resolveBoardDrop(columns, "failed-run", "waiting-run")).toEqual({
      kind: "park",
      runId: "failed-run",
    });
  });

  test("a human-needed run dropped on the blocked column parks", () => {
    const humanRun = boardRun("human-run", "pending");
    humanRun.lifecycle.status = { kind: "human_needed" };
    const built = buildBoardColumns({ data: [humanRun] }, false);
    expect(resolveBoardDrop(built, "human-run", "column:blocked")).toEqual({
      kind: "park",
      runId: "human-run",
    });
  });

  test("a dead run is not parkable: the drop is not a park gesture", () => {
    const deadRun = boardRun("dead-run", "failed");
    deadRun.lifecycle.status = { kind: "dead", reason: "workflow_error" };
    const columns = columnsFor([deadRun]);
    expect(resolveBoardDrop(columns, "dead-run", "column:blocked")).toBeNull();
  });

  test("an active run dropped on the blocked column is not a park gesture", () => {
    const columns = columnsFor([boardRun("running-run", "running")]);
    expect(resolveBoardDrop(columns, "running-run", "column:blocked")).toBeNull();
  });

  test("an owner-parked run dropped on the failed column unparks", () => {
    const columns = columnsFor([
      ownerParkedRun("parked-run"),
      boardRun("failed-run", "failed"),
    ]);
    expect(resolveBoardDrop(columns, "parked-run", "column:failed")).toEqual({
      kind: "unpark",
      runId: "parked-run",
    });
  });

  test("an owner-parked run dropped on a card in the human_needed column unparks", () => {
    const humanRun = boardRun("human-run", "pending");
    humanRun.lifecycle.status = { kind: "human_needed" };
    const columns = columnsFor([ownerParkedRun("parked-run"), humanRun]);
    expect(resolveBoardDrop(columns, "parked-run", "human-run")).toEqual({
      kind: "unpark",
      runId: "parked-run",
    });
  });

  test("an owner-parked run dropped on succeeded is not an unpark gesture", () => {
    const columns = columnsFor([
      ownerParkedRun("parked-run"),
      boardRun("done-run", "succeeded"),
    ]);
    expect(resolveBoardDrop(columns, "parked-run", "done-run")).toBeNull();
  });

  test("a quota-parked run dropped on failed is not an unpark gesture", () => {
    const columns = columnsFor([
      boardRun("quota-run", "blocked"),
      boardRun("failed-run", "failed"),
    ]);
    expect(resolveBoardDrop(columns, "quota-run", "column:failed")).toBeNull();
  });

  test("drops inside one column stay reorder gestures, not park gestures", () => {
    const columns = columnsFor([
      boardRun("failed-a", "failed"),
      boardRun("failed-b", "failed"),
      ownerParkedRun("parked-run"),
    ]);
    expect(resolveBoardDrop(columns, "failed-a", "failed-b")).toBeNull();
    expect(resolveBoardDrop(columns, "parked-run", "column:blocked")).toBeNull();
  });

  test("an unknown drop target resolves to no action", () => {
    const columns = columnsFor([boardRun("failed-run", "failed")]);
    expect(resolveBoardDrop(columns, "failed-run", "column:succeeded")).toBeNull();
    expect(resolveBoardDrop(columns, "ghost-run", "column:blocked")).toBeNull();
  });
});


// ---------------------------------------------------------------------------
// Runs board column menu: "Remove all" beside "Archive all" (work#1664).
// The menu item composes the existing bulk delete (deleteRuns + canDelete)
// behind the destructive-action confirm dialog; the tests stub
// @headlessui/react (the automations render-test convention — plain
// divs/buttons, so the menu body always renders) and partial-mock
// ../lib/run-actions so deleteRuns is observable. bun's mock.module is
// process-wide (restore does not undo it) — spread the REAL module;
// override only deleteRuns.
const deleteRunsMock = mock(() =>
  Promise.resolve({
    results: [],
    summary: { requested: 0, succeeded: 0, failed: 0 },
  }),
);
const realRunActions = await import("../lib/run-actions");
mock.module("../lib/run-actions", () => ({
  ...realRunActions,
  deleteRuns: deleteRunsMock,
}));

const realHeadlessui = await import("@headlessui/react");
mock.module("@headlessui/react", () => ({
  ...realHeadlessui,
  Menu: ({ children }: any) => createElement("div", null, children),
  MenuButton: ({ children, ...props }: any) =>
    createElement("button", props, children),
  MenuItems: ({ children, ...props }: any) =>
    createElement("div", props, children),
  MenuItem: ({ children, ...props }: any) =>
    createElement("div", props, children),
  Dialog: ({ open, children }: any) =>
    open ? createElement("div", { role: "dialog" }, children) : null,
  DialogPanel: ({ children, ...props }: any) =>
    createElement("div", props, children),
  DialogTitle: ({ children, ...props }: any) =>
    createElement("h2", props, children),
}));

const { ColumnActionsMenu } = await import("./runs");

function menuItem(id: string, lifecycleStatus: NonNullable<RunItem["lifecycleStatus"]>): RunItem {
  return {
    id,
    repo: "repo",
    title: `Run ${id}`,
    workflow: "Test",
    createdBy: TEST_PRINCIPAL,
    lifecycleStatus,
  };
}

function menuColumn(items: RunItem[]): Parameters<typeof ColumnActionsMenu>[0]["column"] {
  return {
    id: "succeeded" as BoardColumn,
    name: "Succeeded",
    dot: "bg-teal-300",
    text: "text-teal-300",
    actions: [],
    items,
  };
}

/** The toast mirror (toast.test.tsx convention): reads the Sonner store. */
function SonnerToastText() {
  const { toasts } = useSonner();
  return (
    <output aria-live="polite">
      {toasts.map((toast) => (
        <p key={toast.id}>
          {typeof toast.title === "function" ? toast.title() : toast.title}
        </p>
      ))}
    </output>
  );
}

const mountedRenderers: TestRenderer.ReactTestRenderer[] = [];
let teardownReactEnv: (() => void) | undefined;

function renderMenu(
  column: ReturnType<typeof menuColumn>,
  withToasts = true,
): TestRenderer.ReactTestRenderer {
  let renderer!: TestRenderer.ReactTestRenderer;
  act(() => {
    renderer = TestRenderer.create(
      <>
        {withToasts && <SonnerToastText />}
        <ColumnActionsMenu column={column} />
      </>,
    );
  });
  mountedRenderers.push(renderer);
  return renderer;
}

function buttonsOf(renderer: TestRenderer.ReactTestRenderer) {
  return renderer.root.findAll((node) => node.type === "button");
}

function buttonByLabelPrefix(
  renderer: TestRenderer.ReactTestRenderer,
  prefix: string,
): TestRenderer.ReactTestInstance {
  const matches = buttonsOf(renderer).filter((node) =>
    textContent(node).startsWith(prefix),
  );
  expect(matches.length).toBe(1);
  return matches[0]!;
}

async function clickButton(instance: TestRenderer.ReactTestInstance) {
  await act(async () => {
    instance.props.onClick();
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

async function flushSonnerUpdates() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

describe("runs board column menu: Remove all (work#1664)", () => {
  beforeEach(() => {
    teardownReactEnv = setupReactTestEnv();
    deleteRunsMock.mockClear();
  });

  afterEach(() => {
    for (const renderer of mountedRenderers.splice(0)) {
      act(() => renderer.unmount());
    }
    sonnerToast.dismiss();
    teardownReactEnv?.();
    teardownReactEnv = undefined;
  });

  test("the menu renders Remove all only when the column holds deletable runs", () => {
    // Neither archivable nor deletable: the whole menu stays hidden
    // (the archive arm's null-shape, now over both arms).
    const inert = renderMenu(menuColumn([menuItem("live-1", "running")]), false);
    expect(inert.toJSON()).toBeNull();

    // Terminal runs are deletable AND archivable: both items show.
    const both = renderMenu(
      menuColumn([menuItem("done-1", "succeeded"), menuItem("live-2", "running")]),
    );
    const bothJson = JSON.stringify(both.toJSON());
    expect(bothJson).toContain("Archive all");
    expect(bothJson).toContain("Remove all");
    act(() => both.unmount());

    // The archived column shape: archived runs are deletable but not
    // archivable — only Remove all shows.
    const archivedOnly = renderMenu(menuColumn([menuItem("gone-1", "archived")]));
    const archivedJson = JSON.stringify(archivedOnly.toJSON());
    expect(archivedJson).not.toContain("Archive all");
    expect(archivedJson).toContain("Remove all");
    act(() => archivedOnly.unmount());
  });

  test("the Remove all click never fires: the confirm dialog gates the delete", async () => {
    const renderer = renderMenu(
      menuColumn([menuItem("done-1", "succeeded"), menuItem("gone-1", "archived")]),
    );
    const removeAll = buttonByLabelPrefix(renderer, "Remove all");
    await clickButton(removeAll);

    expect(deleteRunsMock).toHaveBeenCalledTimes(0);
    const json = JSON.stringify(renderer.toJSON());
    // The dialog names the column and the run count.
    expect(json).toContain("Delete 2 runs in Succeeded permanently?");
    expect(json).toContain("unrecoverable");
  });

  test("confirm fires deleteRuns with exactly the deletable ids", async () => {
    deleteRunsMock.mockResolvedValueOnce({
      results: [],
      summary: { requested: 3, succeeded: 3, failed: 0 },
    });
    const renderer = renderMenu(
      menuColumn([
        menuItem("del-1", "succeeded"),
        menuItem("keep-1", "running"),
        menuItem("del-2", "failed"),
        menuItem("del-3", "dead"),
      ]),
    );
    await clickButton(buttonByLabelPrefix(renderer, "Remove all"));
    await clickButton(buttonByLabelPrefix(renderer, "Delete runs"));

    expect(deleteRunsMock).toHaveBeenCalledTimes(1);
    expect(deleteRunsMock.mock.calls[0][0]).toEqual(["del-1", "del-2", "del-3"]);
    await flushSonnerUpdates();
    expect(JSON.stringify(renderer.toJSON())).toContain("Deleted 3 runs.");
  });

  test("cancel closes the dialog without firing", async () => {
    const renderer = renderMenu(menuColumn([menuItem("done-1", "succeeded")]));
    await clickButton(buttonByLabelPrefix(renderer, "Remove all"));
    expect(JSON.stringify(renderer.toJSON())).toContain("permanently?");

    await clickButton(buttonByLabelPrefix(renderer, "Cancel"));
    expect(deleteRunsMock).toHaveBeenCalledTimes(0);
    expect(JSON.stringify(renderer.toJSON())).not.toContain("permanently?");
  });

  test("the in-flight delete relabels Removing N… and disables the dialog until it settles", async () => {
    let settle!: (value: { results: never[]; summary: { requested: number; succeeded: number; failed: number } }) => void;
    deleteRunsMock.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          settle = resolve;
        }),
    );
    const renderer = renderMenu(
      menuColumn([menuItem("done-1", "succeeded"), menuItem("done-2", "failed")]),
    );
    await clickButton(buttonByLabelPrefix(renderer, "Remove all"));
    await clickButton(buttonByLabelPrefix(renderer, "Delete runs"));

    const inflight = JSON.stringify(renderer.toJSON());
    expect(inflight).toContain("Removing 2…");
    expect(inflight).toContain("Deleting…");
    for (const button of buttonsOf(renderer)) {
      expect(button.props.disabled).toBe(true);
    }
    expect(deleteRunsMock).toHaveBeenCalledTimes(1);

    await act(async () => {
      settle({ results: [], summary: { requested: 2, succeeded: 2, failed: 0 } });
    });
    await flushSonnerUpdates();
    expect(JSON.stringify(renderer.toJSON())).toContain("Deleted 2 runs.");
    expect(JSON.stringify(renderer.toJSON())).not.toContain("permanently?");
  });

  test("the partial and total failure paths toast through the Delete summarizer", async () => {
    // Partial: one of two deletions fails.
    deleteRunsMock.mockResolvedValueOnce({
      results: [],
      summary: { requested: 2, succeeded: 1, failed: 1 },
    });
    const partial = renderMenu(
      menuColumn([menuItem("done-1", "succeeded"), menuItem("done-2", "failed")]),
    );
    await clickButton(buttonByLabelPrefix(partial, "Remove all"));
    await clickButton(buttonByLabelPrefix(partial, "Delete runs"));
    await flushSonnerUpdates();
    expect(JSON.stringify(partial.toJSON())).toContain(
      "Deleted 1 of 2 runs. 1 failed.",
    );
    act(() => partial.unmount());

    // Total: the request itself rejects — every run counts as failed.
    deleteRunsMock.mockRejectedValueOnce(new Error("server down"));
    const total = renderMenu(
      menuColumn([menuItem("done-3", "succeeded"), menuItem("done-4", "failed")]),
    );
    await clickButton(buttonByLabelPrefix(total, "Remove all"));
    await clickButton(buttonByLabelPrefix(total, "Delete runs"));
    await flushSonnerUpdates();
    expect(JSON.stringify(total.toJSON())).toContain(
      "Couldn't delete 2 runs. Try again.",
    );
    act(() => total.unmount());
  });
});
