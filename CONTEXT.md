# fabro lab

This repo hosts the gofib product and the Fabro develop workflow that builds
it. Work is split into two worlds, each with its own branch and seed tracker.

## Language

**Product branch**:
`denkhaus-lab` — product code plus product seeds.
_Avoid_: "main branch", "project branch" (ambiguous about which world).

**Platform meta branch**:
`meta/denkhaus-lab` — workflow assets, platform seeds, and the mailbox; the
bird's-eye view of how work flows, never the product content itself.
_Avoid_: "meta" for anything that is not this branch.

**Product seed**:
A seed in the product tracker (`fabro-*`); it builds or verifies product
code. _Avoid_: calling platform work a "product fix".

**Platform seed**:
A seed in the platform tracker (`fabro-meta-*`); it changes workflow assets
(greps, prompts, scripts, gate). _Avoid_: slipping platform scope into a
product seed.

**Painpoint**:
A stage's observation about friction in the flow — evidence gaps, broken
scripts, gate blind spots. The only legal channel for platform criticism
during a product run; never a fix applied in place.
_Avoid_: "issue", "bug report" (those live in trackers).

**Mailbox**:
`painpoints.jsonl` on the platform meta branch — append-only JSONL that
collects delivered painpoints. German: Briefkasten.

**Refiner**:
The workflow step that lifts painpoints from run context into the mailbox
at the end of a run.

**Quality gate**:
The opaque `just qualitygate` contract each implementer answers to. The
product gate checks product code; the platform gate lints workflow assets.
_Avoid_: "tester", "CI".

**Evidence**:
The deterministic capture (diff since run base, worktree, tracker, seed
specs) that feeds the tool-less reviewer. _Avoid_: mixing evidence claims
into implementation summaries.

**Verification-only brief**:
A brief marking a seed whose acceptance criteria look already satisfied;
the implementer verifies without changing code.
