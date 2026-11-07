import type { ReactNode } from "react";

import { GitPullRequestIcon } from "./icons";

/** The state of a run's pull request as the board carries it. Absent
 * means unknown — the list projection does not record PR state yet
 * (denkhaus/work#1164); the colour map below activates the moment it
 * does, without another UI change. */
export type PullRequestState = "open" | "merged" | "closed";

/** The chip owns the PR state colour map (denkhaus/work#1164): merged is
 * done (mint), open is active (cyan), closed is not a failure (muted —
 * never coral). Unknown keeps the default muted look. */
const stateTextClass: Record<PullRequestState, string> = {
  merged: "text-mint",
  open: "text-cyan-500",
  closed: "text-fg-muted",
};

const defaultClassName = "inline-flex items-center gap-1.5 font-mono text-xs text-fg-muted";

export function PullRequestChip({
  number,
  url,
  state,
  className = state == null
    ? defaultClassName
    : defaultClassName.replace("text-fg-muted", stateTextClass[state]),
  iconClassName = "size-3",
  children,
}: {
  number: number;
  url?: string;
  state?: PullRequestState;
  className?: string;
  iconClassName?: string;
  children?: ReactNode;
}) {
  const content = (
    <>
      <GitPullRequestIcon className={iconClassName} />
      {`#${number}`}
      {children}
    </>
  );

  if (url == null) {
    return <span className={className}>{content}</span>;
  }

  return (
    <a
      href={url}
      target="_blank"
      rel="noreferrer"
      className={`${className} hover:text-fg`}
    >
      {content}
    </a>
  );
}
