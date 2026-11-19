import { useState } from "react";
import { useNavigate } from "react-router";
import { useSWRConfig } from "swr";
import {
  ArrowPathIcon,
  ArrowUturnRightIcon,
  SparklesIcon,
  TrashIcon,
  XMarkIcon,
} from "@heroicons/react/24/outline";

import { useRun } from "../lib/queries";
import { mutateRunListCaches } from "../lib/board-cache";
import {
  archiveRun,
  canDelete,
  deleteErrorMessage,
  deleteRun,
  retryLatestRun,
  retryRun,
} from "../lib/run-actions";
import type { Run } from "@qltysh/fabro-api-client";
import { ConfirmDialog } from "./ui";

/**
 * The verdict element (denkhaus/work#1193): a run parked in HITL or a
 * failed run renders WHY it stopped (the recorded conclusion, verbatim),
 * a short context line, and the actions a human can take — only then
 * does the human decide. Sits directly below the run summary panel.
 */
export function RunVerdict({ runId }: { runId: string }) {
  const { data: run } = useRun(runId);
  const navigate = useNavigate();
  const { mutate } = useSWRConfig();
  const [busy, setBusy] = useState<null | "retry" | "retry-latest" | "fork" | "archive" | "delete">(null);
  const [error, setError] = useState<string | null>(null);
  const [deleteDialogOpen, setDeleteDialogOpen] = useState(false);

  const status = run?.lifecycle.status;
  if (!run || !status) return null;
  const humanNeeded = status.kind === "human_needed";
  const failed = status.kind === "failed" || status.kind === "dead";
  if (!humanNeeded && !failed) return null;
  const deletable = canDelete(status.kind);

  // Schema shape (RunLifecycle): the recorded stop reason is
  // `lifecycle.error.message` — denkhaus/work#1337; the old
  // `conclusionFailure` key was a ghost field that never rendered.
  const why = run.lifecycle.error?.message ?? null;
  const question = (run as { current_question?: { text?: string } | null }).current_question ?? null;
  const context = humanNeeded
    ? "The workflow stopped here and waits for a human decision. Read the recorded reason, then retry on the current line tip, continue from the checkpoint as a fork, archive it off the board, or delete it permanently."
    : "The run ended with a failure. The recorded reason is below; retry re-runs the failed stage on the files before it, retry on latest re-drives it on the current workflow version and line tip with the checkpoint work carried, fork continues from the last checkpoint, archive clears it off the board, delete removes it for good.";

  const act = async (kind: "retry" | "retry-latest" | "fork" | "archive" | "delete") => {
    setBusy(kind);
    setError(null);
    try {
      if (kind === "retry") {
        const next = await retryRun(runId);
        navigate(`/runs/${next.id}`);
      } else if (kind === "retry-latest") {
        const next = await retryLatestRun(runId);
        navigate(`/runs/${next.new_run_id}`);
      } else if (kind === "archive") {
        // The verdict only shows for terminal or parked runs — cancel is
        // a no-op there. Archive is the disposition that clears the
        // board column.
        await archiveRun(runId);
        navigate("/runs");
      } else if (kind === "delete") {
        // Delete is the irreversible disposition (work#1479): the run
        // record, its sandbox and durable state, salvage windows, and
        // observations die — so the detail route navigates away like
        // archive does, after the board caches are invalidated.
        await deleteRun(runId);
        mutateRunListCaches(mutate);
        navigate("/runs");
      } else {
        const response = await fetch(`/api/v1/runs/${encodeURIComponent(runId)}/fork`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: "{}",
        });
        if (!response.ok) throw new Error(`fork failed (${response.status})`);
        const forked = (await response.json()) as { run?: Pick<Run, "id"> };
        if (forked.run?.id) navigate(`/runs/${forked.run.id}`);
        else navigate(0);
      }
    } catch (caught) {
      if (kind === "delete") {
        setError(deleteErrorMessage(caught));
      } else {
        setError(caught instanceof Error ? caught.message : String(caught));
      }
    } finally {
      setBusy(null);
    }
  };

  return (
    <>
      <div
        className={`rounded-md border px-4 py-3 ${humanNeeded ? "border-amber/40 bg-amber/5" : "border-coral/30 bg-coral/5"}`}
      >
        <p className="font-mono text-[11px] uppercase tracking-wide text-fg-muted">
          verdict — {humanNeeded ? "needs a human decision" : "failed"}
        </p>
        {question?.text != null ? (
          <p className="mt-1.5 text-sm text-fg">{question.text ?? "(the question carries no text)"}</p>
        ) : why ? (
          <p className="mt-1.5 break-words font-mono text-xs text-fg-2">{why}</p>
        ) : (
          <p className="mt-1.5 text-sm text-fg-muted">No reason was recorded.</p>
        )}
        <p className="mt-1.5 text-xs text-fg-3">{context}</p>
        {error && <p className="mt-2 text-xs text-coral">{error}</p>}
        <div className="mt-3 flex flex-wrap gap-2">
          <button
            type="button"
            disabled={busy != null}
            onClick={() => act("retry")}
            className="inline-flex items-center gap-1.5 rounded-md border border-line px-2.5 py-1 text-[11px] font-medium text-fg transition-colors hover:border-line-strong disabled:cursor-not-allowed disabled:opacity-60"
          >
            <ArrowPathIcon className="size-3.5" /> Retry
          </button>
          <button
            type="button"
            disabled={busy != null}
            onClick={() => act("retry-latest")}
            className="inline-flex items-center gap-1.5 rounded-md border border-line px-2.5 py-1 text-[11px] font-medium text-fg transition-colors hover:border-line-strong disabled:cursor-not-allowed disabled:opacity-60"
          >
            <SparklesIcon className="size-3.5" /> Retry on latest
          </button>
          <button
            type="button"
            disabled={busy != null}
            onClick={() => act("fork")}
            className="inline-flex items-center gap-1.5 rounded-md border border-line px-2.5 py-1 text-[11px] font-medium text-fg transition-colors hover:border-line-strong disabled:cursor-not-allowed disabled:opacity-60"
          >
            <ArrowUturnRightIcon className="size-3.5" /> Fork
          </button>
          <button
            type="button"
            disabled={busy != null}
            onClick={() => act("archive")}
            className="inline-flex items-center gap-1.5 rounded-md border border-line px-2.5 py-1 text-[11px] font-medium text-fg-muted transition-colors hover:text-fg disabled:cursor-not-allowed disabled:opacity-60"
          >
            <XMarkIcon className="size-3.5" /> Archive
          </button>
          {deletable && (
            <button
              type="button"
              disabled={busy != null}
              onClick={() => setDeleteDialogOpen(true)}
              className="inline-flex items-center gap-1.5 rounded-md border border-line px-2.5 py-1 text-[11px] font-medium text-coral transition-colors hover:border-coral/50 disabled:cursor-not-allowed disabled:opacity-60"
            >
              <TrashIcon className="size-3.5" /> Delete
            </button>
          )}
        </div>
      </div>
      <ConfirmDialog
        open={deleteDialogOpen}
        title="Delete run permanently?"
        description={
          <>
            This permanently removes the run — its record, its sandbox and durable state, its salvage
            windows, and its observations and journal. The run is unrecoverable: archive keeps the
            record, delete does not.
          </>
        }
        confirmLabel="Delete run"
        pendingLabel="Deleting…"
        pending={busy === "delete"}
        onConfirm={() => void act("delete")}
        onCancel={() => setDeleteDialogOpen(false)}
      />
    </>
  );
}
