# Improve review — run 01M2NJMRX3DW8R4DW8364G3V8N

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 17:08+0000 by revisor `fabro_ask`

---

## What this run actually did (grounding)

From run events and checkpoints: single clean pass, no cycles, no retries, zero tool errors — start → planner → implementer → tester (gate green, 4.2 s) → evidence (0.3 s) → reviewer (Approved, 0 tool calls) → closeout, exiting at 17:04:19 (~3 m total, 144 s active). Seed claimed: **fabro-ac84** (execute fixture batteries in the qualitygate loop-asset tier). Cost **$0.196**: planner $0.0996 (51%), implementer $0.0682 (35%), reviewer $0.0286 (15%). The evidence pipe worked as designed — 3.5 KB capture rendered inline, reviewer judged entirely from context. The friction below is therefore mostly **preamble weight and planner reconnaissance**, not the loop mechanics.

## Recommendations (by expected impact)

**1. Stop feeding the planner the 200-seed firehose — seed fabro-c3b4 (open).**
What happened: the planner's first call (`sd ready --assignee fabro --limit 200`, run events seq 29–31) returned exactly **200 ready issues, 28,145 bytes, tool tail marked `stdout_truncated: true`** — the listing is *at the cap*, so the `--limit 200` rule's own goal (no silent truncation, fabro-c16d) is already violated. The planner then chewed 59.7 s inference (41 % of run wall) and $0.0996 (51 % of cost) largely on this listing plus the 28-run `fabro_runs_list` payload. Change: in `.fabro/workflows/develop/prompts/planner.md` (and the PROJECT_FACTS command table), replace the full listing with a top-N priority view (N≈10); claim selection is priority-ordered, so the tail is dead weight. Complementary: **fabro-55a7** (batch `sd ready` + `sd show` + basis probe into one shell call — this run used 5 sequential shells before the claim). Expected effect: several thousand input tokens and multiple seconds off every planner pass, and the truncation risk disappears.

**2. Role-scope the PROJECT_FACTS sd-command table — seed fabro-52b4 (open).**
What happened: the implementer (21,365 input tokens) carried the full 6-command tracker table and used **zero** sd commands (the brief was complete); the reviewer (18,199 input tokens, 0 tool calls, `cache_read: 0` — single turn, nothing reused) carried the same table it is forbidden to use. Change: in `.fabro/workflows/develop/prompts/implementer.md` keep only `sd show`; in `reviewer.md` drop the table entirely (fabro-52b4's exact split). Expected effect: ~1.5–2 k tokens cut per stage per run, and non-planner roles can't misapply tracker commands they were never meant to run.

**3. Cap the implementer's step-4 policy block — seed fabro-7b2a (open).**
What happened: the implementer spent **56.2 s inference for a 15-line sed edit** to `scripts/qualitygate.nu` (7 shell calls, 3.0 s tool time — inference:tool ≈ 19:1). Its prompt's step 4 is a ~600-word block of inline run citations (01M20T9S8…, 01M1YJ8R82…, fabro-0d56…) that this loop-asset seed never needed. Change: per fabro-7b2a, reduce step 4 in `.fabro/workflows/develop/prompts/implementer.md` to the operative rule (crate-scoped fmt/clippy/nextest via `just verify implementer`, never the gate) and move the war stories to a footnote/`docs` pointer. Expected effect: faster implementer turns and less policy text to misparse on simple seeds.

**4. Forbid `2>/dev/null` probes in the planner — seed fabro-b6f9 (open; recurrence observed).**
What happened: the stale-basis probe (seq 43) was `ls -la .fabro/scripts/ 2>/dev/null; … grep … 2>/dev/null` — exactly the anti-pattern fabro-b6f9 targets. It happened to succeed, but a failing probe here reads as "path doesn't exist," which feeds the superseded-close decision (fabro-d183): a silently failed probe could close a valid seed. Change: one line in `.fabro/workflows/develop/prompts/planner.md` probe-discipline section (never `2>/dev/null`; echo labeled markers between probe arms — this run's probe did use `---` separators, keep that). Expected effect: probe failures become visible; removes a latent wrong-close path in the highest-stakes planner decision.

**5. Print a positive battery line in the gate output — new seed justified.**
What happened: the tester output (193 bytes, from the reviewer's preamble) shows `lint-nu: green`, `loop-asset scripts green`, `GATE GREEN` — but **no trace that the newly wired fixture battery ran**; `check-loop-assets` only prints on failure (diff in checkpoint seq 132). Success is silent, so the gate output cannot prove the fabro-ac84 tier executed. Change: in `scripts/qualitygate.nu`, `check-loop-assets` battery loop, print e.g. `fixture batteries: 1 green (dup-run-check-fixtures.nu)` after the loop. *New-seed justification: fabro-ac84 landed battery execution but no open seed demands a success line (fabro-750c covers gate stdout in *evidence*, a different mechanism; checked against this run's 200-seed listing).* Expected effect: gate-green output becomes self-evidencing for the battery tier — humans and reviewers confirm coverage by reading, not re-running.

**6. Put per-stage cost/time and journal digest in the PR body — seed fabro-1409 (open).**
What happened: this run produced PR #195; from the run projection its body is generated by the postlude (glm-4.7) and will not show that the pass cost $0.196, took ~3 m, had a 4.2 s gate and **zero journal painpoints across all stages** — exactly the "this was a clean, cheap run" signal a reviewer of 15-line loop-asset PRs wants at a glance. Change: implement fabro-1409 in the PR postlude. Expected effect: PR-level visibility of run economics and friction without opening the run view.

## What I could not inspect

The PR #195 body itself (no GitHub access from here) and the raw blob contents of the tester/evidence/closeout outputs beyond what the reviewer preamble embedded; the sd-command table's per-role split state in the current prompt files (they are fs_hide paths for the run's agents; I relied on the full prompt texts embedded in the run projection, which show the table present in all three role prompts).
