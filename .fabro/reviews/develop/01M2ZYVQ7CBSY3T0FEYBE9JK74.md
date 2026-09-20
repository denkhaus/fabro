# Improve review — run 01M2ZYVQ7CBSY3T0FEYBE9JK74

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (5.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-20 18:17+0000 by revisor `fabro_ask`

---

All evidence is in hand. Sources: run events/stage transcripts and journal via `fabro_run_get`/`fabro_run_events`, plus workspace files `scripts/qualitygate.nu`, `scripts/verify.nu`, `.fabro/scripts/dup-run-check.nu`, `.fabro/workflows/develop/scripts/planner-preflight.nu`, `.fabro/workflows/develop/workflow.toml`, `.seeds/issues.jsonl`.

**Run baseline (for impact math):** wall 5m53s, $0.424 LLM (planner $0.160/76.8s, implementer $0.127/147.5s, reviewer $0.137/85.1s — costliest stage), zero retries, zero gate bounces, one seed (`seeds-e218`, a resume) closed. Graph topology itself caused no waste this run — every node single-visit, all deterministic guards idle — so no edge/node changes are warranted; the friction was all in scripts, defaults, and dead mandates.

---

**1. The quality gate was vacuously green — it never saw the crate it was gating.** (error handling / gate design)
- **What happened:** tester ran 6.3s and printed `no crates touched … GATE GREEN` (from run events, tester stage output) over a diff of ~1,728 lines in `crates/seeds` including 16 new tests. Root cause (from workspace files, confirmed by the implementer's journal painpoint): `scripts/qualitygate.nu:48-58` and `scripts/verify.nu:70-88` filter diffs through `^lib/(apps|components|foundation)/` — the fabro-repo layout this workflow was ported from; this repo uses `crates/`. The round-trip suite ran only because the seed spec forced the implementer to run `nextest` itself; the reviewer journaled exactly this gap and had to approve on that basis.
- **Change:** in both `scripts/qualitygate.nu` (`touched-crates`) and `scripts/verify.nu` (`touched`), derive crate paths from the workspace manifest (`members` in `Cargo.toml`) instead of the hardcoded `lib/` regex; add the reviewer's own proposed guard — fail the gate loudly when the seed-work file list is non-empty but the touched-crate set is empty.
- **Expected effect:** every future Rust seed actually gets clippy+nextest gated; a silent class of vacuous "GATE GREEN" approvals disappears.
- **Seed:** none of `seeds-e218` (closed, format core), `seeds-facc`, `seeds-3791` covers gate/verify crate-path derivation — *new seed: gate+verify ported path bug unique to this repo's `crates/` layout, blocks trust in every future gate result.*

**2. The implementer's duplicate-run preflight is dead on this origin — stale default `--base`.** (error handling)
- **What happened:** `nu .fabro/scripts/dup-run-check.nu seeds-e218 --self <run>` returned `degraded: fetch failed: couldn't find remote ref denkhaus` (implementer journal + `dup-run-check.nu:209` — default `--base = "origin/denkhaus"`, the origin repo's value; the remote part `origin` is split off and `denkhaus` is fetched as a ref). The prompt claims the script "checks the merge-target branch named by PROJECT_FACTS by default" (`origin/main`) — it doesn't. The planner preflight worked only because it passes its own `--base`.
- **Change:** one line in `.fabro/scripts/dup-run-check.nu:209`: default `--base` to `origin/main` (or parse the merge-target from PROJECT_FACTS).
- **Expected effect:** the claim-race stopgap gives real signal on every implementer pass instead of a guaranteed degraded verdict; the documented contract stops contradicting the code.
- **Seed:** *new seed: no existing seed touches dup-run-check's ported default; seeds-facc only repoints tracker scripts at the new binary.*

**3. Auto-merge fails on every run — each run will strand an open PR that then poisons in-flight exclusion.** (UX / config)
- **What happened:** run summary shows `auto_merge.status: failed — "Auto merge is not allowed for this repository"`; `.fabro/workflows/develop/workflow.toml:59` sets `auto_merge = true` on the strength of a comment (lines 49-55) whose evidence is from the fabro repo (PRs #102/#107). This is a new repo without `allow_auto_merge` enabled. PR #1 sits unmerged, and the planner prompt's in-flight rule skips any seed with an open run-linked PR — so stranded PRs compound into wrongly skipped seeds on later runs.
- **Change:** either enable "allow auto-merge" on `github.com/denkhaus/seeds`, or set `auto_merge = false` in `.fabro/workflows/develop/workflow.toml:59` (and have the terminal state reflect "PR awaiting human merge").
- **Expected effect:** runs stop ending in a guaranteed-failed merge step; no accumulating open PRs mis-flagging their seeds as in-flight.
- **Seed:** *new seed: workflow-config fix on repo-specific GitHub settings; no existing seed covers it.*

**4. The lesson-capture channel is dead — `ml` is mandated but unusable.** (loop design)
- **What happened:** implementer's `lesson_capture` = "nothing durable — skipped (no `.mulch/` directory; `ml prime`/`ml record` both fail)" while a genuinely durable lesson (the live-tracker vs frozen-fixture race in `crates/seeds/tests/real_fixture.rs`, which already red-lined once this run) survives only as a journal painpoint — exactly the "dies with the seed" failure mode the channel exists to prevent. `AGENTS.md` and implementer.md step 6 still mandate `ml prime`/`ml record` every session.
- **Change:** either commit a `.mulch/` init to this repo (plus toolchain image awareness), or strike the `ml` mandate from `AGENTS.md` and `.fabro/workflows/develop/prompts/implementer.md` step 6 / lesson-capture section until the mulch phase.
- **Expected effect:** durable lessons land somewhere queryable instead of decaying in run journals; implementers stop burning calls on a guaranteed-failing `ml` attempt.
- **Seed:** *new seed: seeds-3791 explicitly defers the mulch CLI "until the mulch phase" and covers sd→seeds cutover only — it does not fix the dead mandate in the interim.*

**5. Preflight mislabels resume-pointers as in-flight; the planner needs a written rule to override it.** (prompting)
- **What happened:** preflight marked `seeds-e218` `in_flight: true` via run `01M2ZW7TT6KKBR8RSS60ZBSEDR` (run events, `output.preflight`) — that run is engine-terminal (failed 17:18:54, no PR), but its branch tip reads `implementer (succeeded)`, and `terminal-tip?` in `planner-preflight.nu:116-127` only proves terminality from a `closeout` tip, a `(#n)` merge, or a `(failed)` tip past a 60-min grace — an infra-dead run with a mid-graph succeeded tip is *never* provably terminal. The seed body itself names this run as the PRIOR ATTEMPT rescue pointer. The planner resolved it correctly but paid a 15.6s `fabro_runs_list` call (126 runs enumerated, events seq 48-49) plus an extra LLM round to do so.
- **Change:** add one paragraph to `.fabro/workflows/develop/prompts/planner.md` step 4: "an `in_flight_run` whose id is cited in the candidate's own seed body (rescue pointer / PRIOR ATTEMPT) is a mislabel — claim as a RESUME; no cross-check needed." (Optionally mirror as a `resume_pointer` annotation in `planner-preflight.nu`'s candidate row.)
- **Expected effect:** every resume-after-failure seed claims in one planner pass without the 126-run cross-check; the preflight's known blind spot (engine-terminal vs git-tip) stops costing a round trip.
- **Seed:** *new seed: no existing seed covers the preflight/prompt resume-pointer interaction.*

**6. Lockfile noise pushes evidence captures over the inline budget and taxes every review.** (tool usage / evidence pipe)
- **What happened:** the evidence capture was 65.3KB (evidence stage output, run events) — above the reviewer's 16KB `preamble_inline_max_kb` — largely because the full `Cargo.lock` diff (~170 lines) rode along; the reviewer had to page the blob with 9 `read_file` + 1 `grep` calls (reviewer stage transcript) on its way to 84.6s inference / $0.137, the costliest stage of the run.
- **Change:** in `.fabro/workflows/develop/scripts/evidence.nu`, collapse `Cargo.lock` (and any `*.lock`) diffs to a one-line `+N/-N (lockfile, elided)` entry in the capture.
- **Expected effect:** typical captures stay under the 16KB inline ceiling; reviews drop most of their blob-paging tool rounds (the reviewer itself journaled blob paging as a workaround-painpoint class).
- **Seed:** *new seed: evidence-capture formatting change; no existing seed covers it.*

**7. Bound the planner's `fabro_runs_list` in-flight check.** (tool usage)
- **What happened:** the mandatory pre-claim check enumerated all 126 develop runs (15.6s tool call, events seq 48-49) because the prompt says to call it with only `workflow: "develop"` — the tool supports `created_since`, and the preflight's own branch-scan complement is already bounded to 14 days (`planner-preflight.nu:186`).
- **Change:** one clause in `.fabro/workflows/develop/prompts/planner.md` step 4: pass `created_since` matching the preflight's 14-day window.
- **Expected effect:** same exclusion coverage at a fraction of the payload/latency; planner pre-claim rounds get cheaper on every run, not just resumes.
- **Seed:** *new seed: prompt-efficiency tweak, no existing seed covers it.*

One item I could not inspect: whether the GitHub repo setting `allow_auto_merge` can be flipped from within this workspace (recommendation 3 offers both the repo-side and the `workflow.toml`-side fix for that reason).
