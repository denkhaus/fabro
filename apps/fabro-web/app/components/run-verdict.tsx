import { useState } from "react";
import { useNavigate } from "react-router";
import { ArrowPathIcon, ArrowUturnRightIcon, XMarkIcon } from "@heroicons/react/24/outline";

import { useRun } from "../lib/queries";
import { cancelRun, mapError, retryRun } from "../lib/run-actions";
import type { Run } from "@qltysh/fabro-api-client";

/**
 * The verdict element (denkhaus/work#1193): a run parked in HITL or a
 * failed run renders WHY it stopped (the recorded conclusion, verbatim),
 * a short context line, and the actions a human can take — only then
 * does the human decide. Sits directly below the run summary panel.
 */
export function RunVerdict({ runId }: { runId: string }) {
  const { data: run } = useRun(runId);
  const navigate = useNavigate();
  const [busy, setBusy] = useState<null | "retry" | "fork" | "cancel">(null);
  const [error, setError] = useState<string | null>(null);

  const status = run?.lifecycle.status;
  if (!run || !status) return null;
  const humanNeeded = status.kind === "human_needed";
  const failed = status.kind === "failed" || status.kind === "dead";
  if (!humanNeeded && !failed) return null;

  const conclusion = (run.lifecycle as { conclusionFailure?: { reason?: string; detail?: { message?: string } } }).conclusionFailure;
  const why = conclusion?.detail?.message ?? conclusion?.reason ?? null;
  const question = (run as { current_question?: { text?: string } | null }).current_question ?? null;
  const context = humanNeeded
    ? "The workflow stopped here and waits for a human decision. Read the recorded reason, then retry on the current line tip, continue from the checkpoint as a fork, or cancel."
    : "The run ended with a failure. The recorded reason is below; retry re-runs the failed stage on the files before it, fork continues from the last checkpoint.";

  const act = async (kind: "retry" | "fork" | "cancel") => {
    setBusy(kind);
    setError(null);
    try {
      if (kind === "retry") {
        const next = await retryRun(runId);
        navigate(`/runs/${next.id}`);
      } else if (kind === "cancel") {
        await cancelRun(runId);
        navigate(0);
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
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setBusy(null);
    }
  };

  return (
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
          onClick={() => act("fork")}
          className="inline-flex items-center gap-1.5 rounded-md border border-line px-2.5 py-1 text-[11px] font-medium text-fg transition-colors hover:border-line-strong disabled:cursor-not-allowed disabled:opacity-60"
        >
          <ArrowUturnRightIcon className="size-3.5" /> Fork
        </button>
        <button
          type="button"
          disabled={busy != null}
          onClick={() => act("cancel")}
          className="inline-flex items-center gap-1.5 rounded-md border border-line px-2.5 py-1 text-[11px] font-medium text-fg-muted transition-colors hover:text-fg disabled:cursor-not-allowed disabled:opacity-60"
        >
          <XMarkIcon className="size-3.5" /> Cancel
        </button>
      </div>
    </div>
  );
}
