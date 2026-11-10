/**
 * Colored badge styles for the tracker's label vocabulary
 * (denkhaus/work#1235, user review 2026-11-10): type and priority get
 * color gradations — bug red, task blue; prio shades from p0 red to
 * p4 green; the observation categories badge like the triage states.
 * The palette rides the theme tokens (coral/amber/ice/teal/mint).
 */

const BADGE_BASE =
  "rounded-full border px-1.5 py-0.5 font-mono text-[11px] tracking-wide";

const TYPE_STYLES: Record<string, string> = {
  bug: "border-coral/50 text-coral",
  task: "border-teal-500/50 text-teal-300",
  feature: "border-mint/50 text-mint",
  epic: "border-amber/50 text-amber",
};

const PRIO_STYLES: Record<string, string> = {
  p0: "border-coral/60 text-coral",
  p1: "border-coral/40 text-amber",
  p2: "border-line text-ice-300",
  p3: "border-teal-500/40 text-teal-500",
  p4: "border-mint/50 text-mint",
};

const CATEGORY_STYLES: Record<string, string> = {
  painpoint: "border-amber/50 text-amber",
  note: "border-line text-fg-3",
};

/** `type:bug` → red, `type:task` → blue, … unknown types stay neutral. */
export function typeBadgeClass(type: string): string {
  return `${BADGE_BASE} ${TYPE_STYLES[type] ?? "border-line text-fg-muted"}`;
}

/** `p0` → red through `p4` → green; unknown priorities stay neutral. */
export function prioBadgeClass(prio: string): string {
  return `${BADGE_BASE} ${PRIO_STYLES[prio] ?? "border-line text-fg-muted"}`;
}

/** `painpoint` → amber, `note` → neutral — the observation type badge. */
export function categoryBadgeClass(category: string): string {
  return `${BADGE_BASE} ${CATEGORY_STYLES[category] ?? "border-line text-fg-muted"}`;
}

/**
 * Styles a raw tracker label for the detail view's badge row: the
 * `prio:`/`type:` families take their colors, everything else stays
 * the neutral label chip.
 */
export function labelBadgeClass(label: string): string {
  if (label.startsWith("prio:")) {
    return prioBadgeClass(label.slice("prio:".length));
  }
  if (label.startsWith("type:")) {
    return typeBadgeClass(label.slice("type:".length));
  }
  return `${BADGE_BASE} border-line text-fg-muted`;
}
