import { useState } from "react";
import { PencilSquareIcon, TrashIcon } from "@heroicons/react/24/outline";
import { useSWRConfig } from "swr";

import AskFabroConversation from "../components/chats/ask-fabro-conversation";
import { useToast } from "../components/toast";
import { EmptyState } from "../components/state";
import { useAskSessionIndex, useLatestRunId } from "../lib/queries";
import { queryKeys } from "../lib/query-keys";
import { deleteAskSession } from "../lib/ask-index";
import { formatRelativeTime } from "../lib/format";

/**
 * The global supervisor chat (ADR-0046, denkhaus/work#1452): a
 * viewport-filling conversation surface in the main menu, beside work and
 * observations. No run anchor — the conversation determines the context.
 * The left column is the resume picker over the server-side session index
 * (every conversation across runs, per-run docked ones included); picking
 * one resumes it with its full transcript. "New conversation" is the reset
 * door: a fresh session starts read-only and the old one stays readable.
 * Each row also carries a per-session delete affordance (work#1535): one
 * click deletes the conversation server-side and drops it from the list.
 */
export const handle = { hideHeader: true, fullHeight: true, wide: true };

export function meta() {
  return [{ title: "Ask Fabro — Fabro" }];
}

export default function Ask() {
  const [fresh, setFresh] = useState(false);
  const index = useAskSessionIndex();
  const { mutate } = useSWRConfig();
  const { push } = useToast();

  const sessions = index.data ?? [];
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const selected =
    sessions.find((session) => session.id === selectedId) ?? null;

  // Explicit deletion (work#1535): confirm-less by demand — the door
  // removes the session server-side, the refetched index drops the row.
  async function handleDelete(sessionId: string) {
    try {
      await deleteAskSession(sessionId);
      await mutate(queryKeys.ask.index());
      if (selectedId === sessionId) {
        setSelectedId(null);
      }
    } catch {
      push({ message: "Deleting the conversation failed.", tone: "error" });
    }
  }

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
            sessions.map((session) => {
              const title = session.title || "Ask Fabro";
              return (
                <div
                  key={session.id}
                  className={[
                    "group relative flex items-start rounded-md transition-colors",
                    !fresh && session.id === selectedId
                      ? "bg-overlay before:absolute before:inset-y-2 before:left-0 before:w-0.5 before:rounded-full before:bg-teal-500"
                      : "hover:bg-overlay/60",
                  ].join(" ")}
                >
                  <button
                    type="button"
                    onClick={() => {
                      setFresh(false);
                      setSelectedId(session.id);
                    }}
                    className="min-w-0 flex-1 rounded-md px-3 py-2 text-left text-sm focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-teal-500"
                  >
                    <span
                      className={[
                        "block truncate",
                        !fresh && session.id === selectedId
                          ? "text-fg"
                          : "text-fg-3 group-hover:text-fg",
                      ].join(" ")}
                    >
                      {title}
                    </span>
                    <span className="mt-0.5 block truncate font-mono text-xs text-fg-muted">
                      {session.run_id}
                      {session.capability === "full_access" ? " · supervisor" : ""}
                    </span>
                    <span className="block text-xs text-fg-muted">
                      {formatRelativeTime(session.updated_at)}
                    </span>
                  </button>
                  <button
                    type="button"
                    aria-label={`Delete conversation ${title}`}
                    title="Delete conversation"
                    onClick={(event) => {
                      event.stopPropagation();
                      void handleDelete(session.id);
                    }}
                    className="m-1.5 inline-flex size-6 shrink-0 items-center justify-center rounded-md text-fg-muted opacity-0 transition-all hover:bg-overlay-strong hover:text-fg focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-teal-500 group-hover:opacity-100"
                  >
                    <TrashIcon className="size-3.5" />
                  </button>
                </div>
              );
            })
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
