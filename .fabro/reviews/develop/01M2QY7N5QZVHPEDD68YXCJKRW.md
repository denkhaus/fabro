# Improve review — run 01M2QY7N5QZVHPEDD68YXCJKRW

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-17 15:10+0000 by revisor `fabro_ask`

---

## What this run actually was

From run events and checkpoints: a **$0.20 / ~3-minute no-op**. Flow: `preflight` died at 1.4s (deterministic nu parse error) → `planner@1` claimed **fabro-fb06** (80.6s, $0.116 — 58% of run cost, 6 LLM rounds re-deriving the dead preflight's job by hand) → `implementer@1` bounced on a dup-run-check `duplicate` verdict (32.3s, $0.045, zero worktree changes) → `planner@2` superseded-closed fb06 (31.2s, $0.040). Planner stages = 77.5% of the $0.201 total. Final diff: journal + tracker only (PR #219).

**Critical finding first (verified against the workspace):** the run's conclusion was wrong. I grepped the worktree (run branch, base `74202fc5` which *contains* `fdc99ce6`) — the green-probe rule the run closed fb06 over exists **nowhere**: no "green-probe", no "runnable test command", no "cheapest named probe" in any prompt or script under `.fabro/workflows/develop/`. Commit `fdc99ce6` is a revisor *filing* commit ("Revisor: review run …, file seed fabro-fb06 (#218)") that dup-run-check classified `filed_only:false` → false `duplicate` → **wrongful superseded-close of an unimplemented seed**.

## Recommendations, by expected impact

**1. Fix dup-run-check's filed-only classifier and reopen fabro-fb06 — seed fabro-0d48 (open, High).**
This run is a fresh, *worse* instance of fabro-0d48: the subject ", file seed fabro-fb06" matches neither classifier branch, the false `duplicate` didn't just near-Block this time — planner@2 applied a superseded-close, silently deleting an unimplemented seed from the backlog (verified absent above). Change: extend the subject classifier in `.fabro/scripts/dup-run-check.nu` (~line 69, the comma-separated singular "file seed <id>" shape) + pin the fixture, exactly as fabro-0d48 prescribes; separately reopen fb06 (user action). Effect: revisor filing commits stop driving duplicate verdicts and wrongful closes.

**2. Land fabro-2ade (preflight parse fix) immediately after #1 — seed fabro-2ade (open, Medium — recommend priority bump).**
Third consecutive run dead-lands at `planner-preflight.nu:135` (`mut closed = {seed: null, sha: null}` → type_mismatch at :152); this run + sibling `01M2QY64MN` + basis run. Because the deterministic short-circuit was dead, the loop spent $0.201 across 3 LLM laps on what a working preflight exits in ~30s for $0 (the fabro-a32f case), and the 597-byte error dump rode every downstream preamble. One-line change in `.fabro/workflows/develop/scripts/planner-preflight.nu:135`. Sequencing per the seed's own note: **after** fabro-0d48 — a revived preflight inheriting the false-duplicate would have wrongfully closed fb06 with zero LLM review at all.

**3. Fix the implementer's Blocked output template — same drift class as fabro-dd22 (open), but a different node; new-seed justification: fabro-dd22 covers only the revisor analyze.md contract mismatch, and no tracker seed names implementer.md's Blocked JSON.**
Event seq 94–99: the implementer's first final answer emitted `"preferred_next_label": "Blocked"` exactly as `implementer.md`'s Blocked template instructs; the engine rejected it ("not one of this node's outgoing edge labels: Implemented"), burning a retry round (~11s, $0.009) where the model had to guess to omit the field. The `implementer → planner` edge fires on `outcome=failed`; the label is not emittable. Change: in `.fabro/workflows/develop/prompts/implementer.md`, the Blocked example must omit `preferred_next_label`. Effect: every duplicate-bounce Blocked route (frequent, per the claim-race stopgap) saves one validation-retry LLM round.

**4. Planner degraded-mode: mandate dup-run-check when `output.preflight` is absent — new-seed justification: fabro-0da8 proposes a new guard node and fabro-a32f (closed) built the preflight; no open seed covers the *degraded-mode fallback* inside the planner's ALREADY-LANDED arm.**
Planner@1 eyeballed `fdc99ce6`'s subject and journaled "not an implementing fix"; the implementer's tool said `duplicate` minutes later — two LLM stages disagreed because the planner did judgment where a deterministic tool exists, and the recovery (implementer + planner@2) cost ~63s / $0.085. Change: `.fabro/workflows/develop/prompts/planner.md`, ALREADY LANDED arm — "when `output.preflight` is absent/degraded, run `nu .fabro/scripts/dup-run-check.nu <id> --self <run-id>` per candidate with base-history refs; never adjudicate landed-ness from commit subjects." Effect: one planner lap resolves duplicates deterministically instead of three laps disagreeing.

**5. Cut the sd-ready firehose — seed fabro-c3b4 (open, Medium).**
Planner@1's first call returned 200 lines / ~28.8KB retained (~7k tokens, stdout-truncated) to pick one seed. Change per fabro-c3b4: top-N view in `.fabro/workflows/develop/prompts/planner.md`, full listing only on fallback. Effect: several k tokens less context per planning pass, fewer distracted tie-breaks.

**6. Let observed-failure-class seeds jump priority tiers — widen fabro-11d3 (open; it covers only same-priority tie-breaks, so a duplicate filing would fail the tracker's own dedup lint — amend it instead).**
The planner journaled the miss itself: fb06 (High, polish) claimed over fabro-2ade (Medium, closes the failure class this run was *actively suffering from*); fb06's brief then had to carry a workaround constraint for 2ade's bug. The run then paid full price for both errors. Change: fabro-11d3's one-line rule in planner.md step 2 extended to "observed failure class outranks polish up to one priority tier." Effect: guard-rail fixes (0d48/2ade) land after first observed failure, not after 3+ dead runs.

**7. Skip the project gate and annotate bookkeeping-only runs — seeds fabro-9495 + fabro-b1d3 (both open).**
PR #219 carries 2 files (+3/−1: journal + tracker) yet runs auto_merge through the full Rust gate (~15min worst case per the tester node's own calibration) — and in Slack/summary a $0.20 no-op reads identically to a delivering run. Change per fabro-9495: loop-asset-only run PRs merge gate-free; per fabro-b1d3: annotate such runs in the run.completed payload. Effect: minutes of CI saved per bookkeeping PR and honest no-op visibility for the user.

**Sources:** run events seq 18–99 and checkpoints 23/75/106/138 (timings, cost, transcripts, journals); workspace greps of `.fabro/workflows/develop/**` and `.seeds/issues.jsonl`. One caveat: I could not inspect commit `fdc99ce6`'s raw diff directly (no shell in this analyst session); the "not implemented" verdict rests on the run-branch worktree greps plus planner@1's own captured step-3 text, which agree.
