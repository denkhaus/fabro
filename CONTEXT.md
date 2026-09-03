# fabro lab — platform world (meta/denkhaus-lab)

This branch hosts the dev loop itself: the develop workflow and its
prompts/scripts, the platform seed tracker, the painpoint mailbox, and the
reviewer-agent. It never hosts product code. Product work lives on the
product branch; see the pointer in its CONTEXT.md.

## Language

**Product branch**:
`denkhaus-lab` — product code plus product seeds.
_Avoid_: "main branch", "project branch" (ambiguous about which world).

**Platform meta branch**:
`meta/denkhaus-lab` — this branch: workflow assets, platform seeds, the
mailbox; the bird's-eye view of how work flows, never the product content.
_Avoid_: "meta" for anything that is not this branch.

**Product seed**:
A seed in the product tracker (`fabro-*`); it builds or verifies product
code. _Avoid_: calling platform work a "product fix".

**Platform seed**:
A seed in this tracker (`fabro-meta-*`); it changes workflow assets
(prompts, scripts, gate, reviewer). _Avoid_: slipping platform scope into
a product seed.

**Painpoint**:
A stage's observation about friction in the flow — evidence gaps, broken
scripts, gate blind spots. The only legal channel for platform criticism
during a product run; never a fix applied in place.
_Avoid_: "issue", "bug report" (those live in trackers).

**Mailbox**:
`painpoints.jsonl` on this branch — append-only JSONL that collects
delivered painpoints. German: Briefkasten.

**Refiner**:
The workflow step that lifts painpoints from run context into the mailbox
at the end of a run.

**Quality gate**:
The opaque `just qualitygate` contract each implementer answers to. The
product gate checks product code; the platform gate lints workflow assets
(fabro validate + nu-check + reviewer-agent, ADR-0004).
_Avoid_: "tester", "CI".

**Evidence**:
The deterministic capture (integrity header, seed spec, seed-work diff,
loop churn, worktree) that feeds the tool-less reviewer.
_Avoid_: mixing evidence claims into implementation summaries.

**Verification-only brief**:
A brief marking a seed whose acceptance criteria look already satisfied;
the implementer verifies without changing code.

**Closeout**:
The deterministic post-approval step: closes the approved seed and routes on
tracker state (Tracker empty / More seeds). Never an LLM. _Avoid_: the
planner closing seeds, acting on approved verdicts (ADR-0010).

**Journal stream**:
`.fabro/journal/<run_id>.jsonl` — one JSON line per stage completion
(`fabro-journal-v1` line shape), one file per run, visits run-local.
_Avoid_: per-node journal files (they collide across runs sharing a base);
unattributable journal lines. See ADR-0009.

**Revision marker**:
`.fabro/revisions/<run-id>.md` — the bookkeeping file a revisor pass writes
after filing; its absence from the base branch is what marks a run
unrevised. _Avoid_: treating the review file as the marker.

**Review (revisor)**:
`.fabro/reviews/<workflow>/<run-id>.md` — the verbatim Ask-Fabro analyst
answer a revisor pass persists for a target run. Feeds distillation; never
the marker itself.

**duplicate_of**:
The analyzer relation: a finding names the same concrete change as an
existing seed (open or closed), so it is dropped before filing; the journal
records `duplicate_of: <id>`. _Avoid_: filing the duplicate and merging
later (ADR-0014).

**supersedes**:
The bookkeeper relation: a new seed replaces the SAME target with a
strictly better solution, so the old open seed is closed with
`sd close --reason "superseded by <new-id>: ..."`. Thematic overlap is not
supersession — cross-reference instead, close nothing. _Avoid_: SUPERSEDES
notes in descriptions without closing (stale open seeds). See ADR-0014.
