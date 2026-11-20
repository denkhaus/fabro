import { useState } from "react";
import { PencilSquareIcon } from "@heroicons/react/24/outline";

import AskFabroConversation from "../components/chats/ask-fabro-conversation";
import { EmptyState } from "../components/state";
import { useAskSessionIndex, useLatestRunId } from "../lib/queries";
import { formatRelativeTime } from "../lib/format";

/**
 * The global supervisor chat (ADR-0046, denkhaus/work#1452): a
 * viewport-filling conversation surface in the main menu, beside work and
 * observations. No run anchor — the conversation determines the context.
 * The left column is the resume picker over the server-side session index
 * (every conversation across runs, per-run docked ones included); picking
 * one resumes it with its full transcript. "New conversation" is the reset
 * door: a fresh session starts read-only and the old one stays readable.
 */
export const handle = { hideHeader: true, fullHeight: true, wide: true };

export function meta() {
  return [{ title: "Ask Fabro — Fabro" }];
}

export default function Ask() {
  const [fresh, setFresh] = useState(false);
  const index = useAskSessionIndex();

  const sessions = index.data ?? [];
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const selected =
    sessions.find((session) => session.id === selectedId) ?? null;

  return (
    // Cancel AppShell's content padding so the chat surface bleeds
    // edge-to-edge below the top nav (mirrors the chats layout).
    <div className="fabro-chat relative isolate -mx-4 -my-6 flex h-[calc(100%+3rem)] sm:-mx-6 lg:-mx-8">
      <aside className="flex w-72 shrink-0 flex-col border-r border-line bg-panel">
        <div className="p-3">
          <button
            type="button"
            onClick={() => {
              setFresh(true);
              setSelectedId(null);
            }}
            className="flex w-full items-center justify-center gap-2 rounded-lg bg-overlay-strong px-3 py-2 text-sm font-medium text-fg ring-1 ring-line-strong transition-all hover:bg-overlay-strong hover:ring-line-strong focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-teal-500"
          >
            <PencilSquareIcon className="size-4" />
            New conversation
          </button>
        </div>
        <nav
          aria-label="Conversations"
          className="flex-1 overflow-y-auto px-2 pt-2 pb-3"
        >
          {index.isLoading && sessions.length === 0 ? (
            <p className="px-3 py-2 text-sm text-fg-3">Loading…</p>
          ) : sessions.length === 0 ? (
            <p className="px-3 py-2 text-sm text-fg-3">
              No conversations yet — start one.
            </p>
          ) : (
            sessions.map((session) => (
              <button
                key={session.id}
                type="button"
                onClick={() => {
                  setFresh(false);
                  setSelectedId(session.id);
                }}
                className={[
                  "relative block w-full rounded-md px-3 py-2 text-left text-sm transition-colors",
                  !fresh && session.id === selectedId
                    ? "bg-overlay text-fg before:absolute before:inset-y-2 before:left-0 before:w-0.5 before:rounded-full before:bg-teal-500"
                    : "text-fg-3 hover:bg-overlay/60 hover:text-fg",
                ].join(" ")}
              >
                <span className="block truncate">
                  {session.title || "Ask Fabro"}
                </span>
                <span className="mt-0.5 block truncate font-mono text-xs text-fg-muted">
                  {session.run_id}
                  {session.capability === "full_access" ? " · supervisor" : ""}
                </span>
                <span className="block text-xs text-fg-muted">
                  {formatRelativeTime(session.updated_at)}
                </span>
              </button>
            ))
          )}
        </nav>
      </aside>
      <div className="min-h-0 flex-1">
        {fresh ? (
          <FreshConversation />
        ) : selected ? (
          <AskFabroConversation
            key={selected.id}
            runId={selected.run_id}
            initialSessionId={selected.id}
          />
        ) : (
          <div className="flex h-full items-center justify-center p-8">
            <EmptyState
              title="Pick a conversation, or start a new one."
              action={null}
            />
          </div>
        )}
      </div>
    </div>
  );
}

/** A brand-new global conversation anchors to the newest live run — the
 * page itself carries no run; the conversation determines the context. */
function FreshConversation() {
  const latestRun = useLatestRunId();
  if (latestRun.isLoading) {
    return (
      <div className="flex h-full items-center justify-center px-4 text-sm text-fg-3">
        Starting conversation…
      </div>
    );
  }
  if (!latestRun.data) {
    return (
      <div className="flex h-full items-center justify-center p-8">
        <EmptyState
          title="No live run to anchor to — a conversation needs a run with a live sandbox."
          action={null}
        />
      </div>
    );
  }
  return (
    <AskFabroConversation
      key={`new-${latestRun.data}`}
      runId={latestRun.data}
      initialSessionId={null}
    />
  );
}
