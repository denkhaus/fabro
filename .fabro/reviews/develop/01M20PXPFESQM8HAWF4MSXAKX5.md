# Improve review — run 01M20PXPFESQM8HAWF4MSXAKX5

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (4.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-08 14:42+0000 by revisor `fabro_ask`

---

Grounded in this run's events, journals, and timings (run 01M20PXPFESQM8HAWF4MSXAKX5: seed fabro-0b41, a 2-hunk prompt-only edit; 4m55s wall, $0.194 total, planner 34% of cost, single-pass green, zero retries). Ordered by expected impact:

## 1. Stop telling the planner to run `gh` — it was deliberately removed
**What happened:** Planner step 4 mandates `gh pr list --state open`. The toolchain image has no `gh` (ADR-0019 user decision, per the Dockerfile comment in the run spec), so the call failed (`gh: command not found`, event seq 41–42), the check degraded to "no in-flight PRs," and the unverified "sequence AFTER PR #59" caveat was pushed into the brief — costing the implementer an extra `git log` round trip and injecting ambiguity into an otherwise clean spec. This burns a dead call **every run** and leaves the double-pick guard (fabro-22e4) permanently disabled.
**Change:** `.fabro/workflows/develop/prompts/planner.md`, step 4 — delete the `gh` instruction; make the primary check the documented credential-less fallback (`fabro_runs_list`, per open seed fabro-06e0) or `git log origin/<base> --oneline` when PR state is what matters.
**Expected effect:** Eliminates one guaranteed-failing tool call + LLM round trip per run, restores a real in-flight-PR guard, and stops sequencing caveats from leaking into implementer briefs.

## 2. Fix evidence-capture ordering vs. the preamble's head-line omission
**What happened:** The reviewer's preamble rendered the evidence output with **"(6 lines omitted)"** — the *first* six lines, exactly where the integrity header and seed-spec/acceptance content sat. The reviewer could not see the staged journal-excerpt verification and re-derived it itself via `sd show fabro-b11a` + rg (its journal painpoint says this verbatim). That's extra tool calls and reasoning on every review, and it is precisely the trigger path for "Verification blocked" cycles.
**Change:** `.fabro/workflows/develop/scripts/evidence.nu` — reorder so the integrity header + seed spec sit *after* the diff and loop-churn sections are last (the stage renderer keeps the tail, not the head); longer term, the engine's stage-output renderer should elide middle lines, not head lines (related open seeds fabro-cf3e, fabro-meta-c9f2).
**Expected effect:** Reviewer judges from context on first pass; ~1 tool round trip + ~10–20s inference saved per review; fewer evidence-delivery rejections.

## 3. Give the planner a top-N view instead of the 169-seed firehose
**What happened:** `sd ready --assignee fabro --limit 200` dumped **22.7 KB / 169 issues** into planner context (event seq 32) — and the chosen seed was the *first line*. The planner still consumed 60.7s inference and $0.066 (34% of run cost, largest stage share with the implementer at $0.089) mostly tie-breaking a list whose order was already deterministic.
**Change:** `.fabro/workflows/develop/prompts/planner.md` step 1 — pipe through `head -15` (open seed fabro-c3b4), or better, add a deterministic pre-planner command node that injects a bounded top-N candidate list as a context key.
**Expected effect:** Cuts planner input tokens (~12k on the first call) and tie-break reasoning; expect planner to drop toward ~40s/$0.04 on claim-only passes.

## 4. Make the fs_hide denial message name the shell bypass
**What happened:** Despite the prompt's "never burn tool calls discovering the denial," the planner still tried `read_file` on `.fabro/workflows/revisor/prompts/file.md`, got *"hidden … behaves as if it did not exist"* (seq 52–53), reasoned "fs_hide. Use shell cat/sed," and only then used `sed`. The denial message doesn't mention the escape hatch, so the prompt's warning loses to the tool's own feedback.
**Change:** Engine fs_hide denial text (one line): append "— use a shell command to access this path" (complements open seed fabro-8296 for glob).
**Expected effect:** One saved LLM round trip on every platform-path touch (planner *and* implementer, both hit `.fabro/**` this run).

## 5. Gate the implementer's `sd show` re-fetch on brief quality
**What happened:** The planner had just written the brief from `sd show --format json` (full FIX text + acceptance criteria), yet implementer step 1 ("Re-read the seed requirements from `sd show`") triggered a redundant re-fetch (seq 90) — same data, one more call and turn of latency. This is the exact behavior open seed fabro-4881 tracks.
**Change:** `.fabro/workflows/develop/prompts/implementer.md`, step 1 — "If the brief already carries the seed's full FIX text and bulleted criteria, skip the re-fetch; fetch only when the brief is thin or a re-plan folded in feedback."
**Expected effect:** One fewer call + ~2.5k input tokens per implementer pass; on a 131s-inference stage, trims the front end of every pass.

## 6. Two smaller, run-verified items
- **Expected warn noise:** `preamble_allow_keys entry absent … key=output.gate_known_bug_hits` fired on the implementer even though the key legitimately doesn't exist on green first passes — downgrade to info (open seed fabro-8275), so real producer drift stays visible in `fabro_run_logs`.
- **Metadata snapshots:** each checkpoint ran a ~2.2s synchronous snapshot (six in this run ≈ 13s ≈ 4% of wall). Making them async/branch-point-only (open seed fabro-cf03) is nearly free wall-time on every run.

**What already worked well (don't touch):** deterministic nodes were essentially free (tester 4.4s with correct "no crates touched" scoping, evidence 0.4s, closeout 0.3s); the gate-bounce enrichment node correctly stayed out of the path on a green run; cache hits were strong (263k cache-read vs 0 write-tokens); the graph's structural cycle guards needed no model compliance and the run exited clean in one pass.
