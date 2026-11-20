import { useId, useMemo } from "react";
import {
  AssistantRuntimeProvider,
  useLocalRuntime,
  type ThreadMessageLike,
} from "@assistant-ui/react";
import { Thread, makeMarkdownText } from "@assistant-ui/react-ui";
import remarkGfm from "remark-gfm";

import { createAskFabroAdapter } from "../../lib/ask-fabro-runtime";
import { useAskSessionTranscript } from "../../lib/queries";
import SidebarComposer from "./sidebar-composer";
import SidebarWelcome from "./sidebar-welcome";
import ToolCallSummary from "./tool-call-summary";

// remark-gfm enables GitHub-flavored Markdown so the agent's tables, task
// lists, and strikethrough render instead of leaking as raw `|` syntax.
const MarkdownText = makeMarkdownText({ remarkPlugins: [remarkGfm] });

/**
 * One Ask Fabro conversation: the assistant-ui Thread bound to a session.
 *
 * `initialSessionId` resumes that session with its full transcript (folded
 * from the session's recorded events); `null` starts a fresh session
 * lazily on the first turn. The reset door (work#1452) works by remounting
 * this component with a changed key and `initialSessionId: null` — the old
 * session stays readable in the index.
 *
 * Keyed by the caller; the session id never changes inside an instance.
 */
export default function AskFabroConversation({
  runId,
  initialSessionId,
  defaultModel,
  onSessionCreated,
}: {
  runId: string;
  initialSessionId: string | null;
  defaultModel?: string | null;
  onSessionCreated?: (sessionId: string) => void;
}) {
  const adapter = useMemo(
    () =>
      createAskFabroAdapter({
        runId,
        initialSessionId,
        defaultModel,
        onSessionCreated,
      }),
    // The component is keyed by session identity; the adapter is built once
    // per instance on purpose — a mid-conversation rebuild would reset the
    // live thread. Callers keep every input stable across the instance's
    // lifetime.
    // react-doctor-disable-next-line react-doctor/exhaustive-deps -- keyed mount-once adapter
    [],
  );
  // The transcript cache key carries a per-mount token (work#1507): every
  // mount of this component is a cache miss, so the fetch below always
  // answers from the events door — a turn that completed while the
  // conversation was unreachable (navigated away mid-turn) shows up on the
  // next visit instead of replaying a stale fold. The component is keyed by
  // the caller per session, so the token is stable for the instance's
  // lifetime and the fetch runs exactly once per mount.
  const mountToken = useId();
  const transcript = useAskSessionTranscript(
    initialSessionId,
    initialSessionId ? mountToken : undefined,
  );
  const initialMessages: readonly ThreadMessageLike[] =
    initialSessionId && transcript.data ? transcript.data : [];
  const runtime = useLocalRuntime(adapter, { initialMessages });

  if (initialSessionId && transcript.isLoading) {
    return (
      <div className="flex h-full items-center justify-center px-4 text-sm text-fg-3">
        Loading conversation…
      </div>
    );
  }

  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <Thread
        components={{ Composer: SidebarComposer, ThreadWelcome: SidebarWelcome }}
        assistantMessage={{
          components: { Text: MarkdownText, ToolFallback: ToolCallSummary },
          allowCopy: false,
          allowReload: false,
          allowSpeak: false,
          allowFeedbackPositive: false,
          allowFeedbackNegative: false,
        }}
      />
    </AssistantRuntimeProvider>
  );
}
