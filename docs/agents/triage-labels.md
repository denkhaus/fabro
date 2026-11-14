# Triage Labels

The skills speak in terms of five canonical triage roles. This file maps those
roles onto the work-item label standard (ADR-0032; see
`docs/agents/issue-tracker.md`): `prio:pN`, `type:*`, `lane:*`, free-form
topic labels. `seeds:*` is legacy-only.

| Label in mattpocock/skills | Our tracker form | Meaning |
| -------------------------- | ---------------- | ------- |
| `needs-triage`             | `needs-triage` (free-form label) AND no `lane:*` label | sits in the human-triage backlog |
| `needs-info`              | `needs-info` (free-form label) | waiting on the reporter for more information |
| `ready-for-agent`         | `lane:develop` | the agent line's candidate pool — fully specified, the line works it |
| `ready-for-agent` (loop assets) | `lane:loop` | same, but the primary fix surface is loop assets |
| `ready-for-human`         | no `lane:*` label (the backlog) | requires human implementation |
| `wontfix`                 | `close --reason "wontfix: …"` | will not be actioned — closed, never a zombie label |

Notes:

- Lanes are the real switch: an OPEN item without a lane IS the
  human-triage queue; `ready-for-agent` is literally `lane:develop`.
- **The lane follows the PRIMARY fix surface.** An item whose primary
  surface is loop assets (`.fabro/**`, root `scripts/**`, `justfile`,
  fabro-test rigs) goes to `lane:loop`, never `lane:develop`: the
  develop line's fs envelope kills those writes at the first
  out-of-scope edit — a `lane:develop` placement burns a grant cycle
  on the misroute every pass until triage moves it (the planner's
  envelope rule releases such items at plan time; work#1335 was the
  twice-per-pass case on 2026-11-14). A repo-visible REMAINDER (docs,
  ADR amendment, `lib/**`) does not make the item develop-able — the
  primary surface decides; split the item instead when the remainder
  is substantive.
- A wontfix is a closed item with the reason, not an open item carrying a
  label — open-but-dead does not exist on this tracker.
- Free-form labels (`needs-triage`, `needs-info`) exist only once
  applied; the tracker has no label registry requirement.
