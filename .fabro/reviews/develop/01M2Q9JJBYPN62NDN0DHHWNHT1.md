# Improve review — run 01M2Q9JJBYPN62NDN0DHHWNHT1

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (3.1 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-17 09:10+0000 by revisor `fabro_ask`

---

All evidence below is from this run's events/checkpoints (run 01M2Q9JJBYPN62NDN0DHHWNHT1, seed fabro-6ae9, 171.5 s wall, $0.227 total: planner 47.4 s/$0.092, implementer 74.9 s/$0.103, reviewer 25.8 s/$0.032, tester 4.8 s), its journal painpoints, and workspace reads of `.fabro/workflows/develop/scripts/planner-preflight.nu` and `.seeds/issues.jsonl`.

**What worked (keep as-is):** the preflight fail-open edge routed the planner around a dead node; evidence (5.9 KB) rendered inline with no blob detour; the reviewer approved with zero tool calls off the inline PASS/FAIL report.

## Recommendations, by expected impact

**1. Fix dup-run-check's filed-only misclassification — it nearly auto-closed a healthy seed.**
What happened: the implementer's mandated preflight (`nu .fabro/scripts/dup-run-check.nu fabro-6ae9 --self 01M2Q9JJ…`, events seq 89–91) returned `verdict: "duplicate", filed_only: false` against commit 706d639 — a revisor **filing** pass ("file seeds fabro-6ae9…(#212)", zero diff to planner.md). The implementer overrode it correctly but burned two extra LLM rounds plus a grep/`git show` probe (seq 93–100, ~19 s / ~$0.036); a compliant model would have routed Blocked and killed the run. Worse: `planner-preflight.nu:126` takes `verdict == "duplicate"` as `landed` and would then write a closure note and `sd close` the seed — every revisor-filed seed has exactly this filing-commit shape.
Change: in `.fabro/scripts/dup-run-check.nu`, classify a match `filed_only` when its diff touches no file named in the seed's target-path spec (or subject matches `/file seed/`), and bar filed-only matches from the `duplicate` verdict; add this exact shape (squash `(#n)` subject of a filing pass) to `.fabro/scripts/dup-run-check-fixtures.nu` so the gate battery pins it (this run's 193-byte gate output shows no battery line at all — see open fabro-8dd8 for making that tier visible).
Expected effect: no false Blocked routes and no wrongful superseded-closes on freshly filed seeds — a correctness fix for the loop's admission path.
Seed: prior art is closed fabro-8c75 (implemented filed-only classification); no open seed tracks this regression — grep of `.seeds/issues.jsonl` for `dup-run-check`/`filed_only` shows the chain closed and open fabro-a01f covers only the missed-duplicate direction, so a new seed is justified as the reopen of the closed carrier.

**2. Fix the parse-time Nushell crash that makes the preflight node dead-on-arrival — but land it with #1.**
What happened: `preflight@1` failed 1.3 s into the run (first-ever execution of the node from PR #209) with `nu::shell::type_mismatch` at `planner-preflight.nu:152` — `mut closed = {seed: null, sha: null}` (line 135, confirmed in the workspace file) infers `record<seed: nothing, sha: nothing>` and rejects the string-record assignment. Planner, implementer, and reviewer all journaled it. Because it's a parse-time error, the node has never produced a verdict table or an Already-landed exit; the planner re-derived those checks by hand (7 tool calls, 6 LLM rounds — the planner was the second-costliest stage at $0.092), and every downstream prompt carried the failed-stage dump.
Change: in `.fabro/workflows/develop/scripts/planner-preflight.nu:135`, initialize `mut closed = {seed: "", sha: ""}` (and test `$closed.seed != ""` at line 161), or build `$closed` as a `let` from the if/else expression.
Expected effect: the fabro-a32f win (93.6 s/$0.143 of planner re-derivation per stale-tracker run) actually materializes; preambles stop carrying a dead node's error. **Ordering constraint:** land after/with recommendation 1, otherwise the revived preflight immediately inherits the false-`duplicate` verdict and starts closing freshly filed seeds.
Seed: new-seed justification — grep of `.seeds/issues.jsonl` for `planner-preflight|type_mismatch|type mismatch` returns zero seeds (fabro-a32f, which introduced the node, is closed); this run's journals carry the finding but nothing is filed.

**3. Skip the Rust gate tier for loop-asset-only diffs.**
What happened: the tester ran `just qualitygate` on a diff of `planner.md` + `.mulch/**` + `.seeds/**` + journal; output said "no crates touched" and finished green in 4.8 s — cheap only because the cache was warm (the graph comment budgets ~15 m cold). What actually guarded this change was the loop-asset lint tier ("lint-nu: green").
Change: in `scripts/qualitygate.nu`, derive touched crates from the per-seed claim base and skip the cargo tier when the diff is loop-asset-only.
Expected effect: removes a redundant (and cold-cache up-to-15-minute) gate lap from every prompt/tracker-only seed.
Seed: existing **fabro-574d** (open, assigned fabro) — exact cover; fabro-9495 is the complementary PR-side variant.

**4. Trim the planner's `sd ready` firehose.**
What happened: the planner's first tool call returned 200 seeds / 28.6 KB retained stdout (event seq 38); only the top High rows were used, and the planner's 38.4k input tokens made it the second-costliest stage. Every lap pays this.
Change: planner PROJECT_FACTS / prompt — top-N (~15) `sd ready` view instead of `--limit 200` full listing.
Expected effect: smaller planner context every run; fewer input tokens and less scanning before the claim.
Seed: existing **fabro-c3b4** (open) — exact cover; fabro-55a7 (batch recon) and fabro-e4fa (sd call economy) are adjacent economies.

**5. Surface fail-open degradation in the run summary / PR body (UX).**
What happened: the run terminated "succeeded" with a green Slack notification to #dev-fabro; the deterministic preflight failure is visible only in checkpoints/journals (three stages journaled it). A user reviewing PR #213 sees nothing about the degraded already-landed guard.
Change: render the checkpoint's `loop_failure_signatures` (already tracked — this run recorded the preflight signature once) as a warning line in the run summary and PR postlude.
Expected effect: degraded guards become visible without journal archaeology.
Seed: existing **fabro-5b0a** (open) — exact cover.

**6. Shrink the implementer prompt by splitting PROJECT_FACTS' sd table per role (prompting).**
What happened: the implementer carried the full six-row sd command table — including claim/close forms it is forbidden to use — plus planner-side arms, and spent 72.8 s inference against 1.9 s tool time for a one-line markdown edit ($0.103, the run's largest stage cost).
Change: per-role PROJECT_FACTS rendering — implementer gets `sd show` only, reviewer none (planner keeps the full table).
Expected effect: smaller per-stage prompts and less inference latency on small seeds; removes forbidden-command noise the reviewer must adjudicate as scope.
Seed: existing **fabro-52b4** (open, assigned fabro) — exact cover; open fabro-7b2a and fabro-b22e attack the same bloat from the step-4 side.

Not inspected: I could not verify whether the improve/revisor workflow has filed seeds for #1/#2 after this run's journals (no post-09:04 filings exist in the tracker snapshot I read); my greps covered all of `.seeds/issues.jsonl`.
