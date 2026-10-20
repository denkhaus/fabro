# Improve review — run 01M5CR2B9DKQ0ZRXKD3DZSPMP0

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (3m39s, revisor pass — reason and cost in run detail)
- generated: 2026-10-20 14:05+0000 by revisor `fabro_ask`

---

# Recommendations — grounded in run 01M5CR2B9DKQ0ZRXKD3DZSPMP0

**Run baseline (from run events / stage telemetry):** wall 3m39s, cost $0.209, planner+implementer = $0.163 (78% of spend). Everything below happened in this run.

---

### 1. Stop the planner brief from naming lint tiers — it re-introduced the double battery
- **What happened:** the implementer's recorded checks (visible in the evidence stage output) show it ran `nu .fabro/scripts/prompt-lint.nu` (81 files), `just lint-nu` (67 scripts), and `check-run-scope` — whole-repo tiers the tester then re-ran verbatim 0.4s later (identical output blocks in tester@1). The single-file-probe rule exists (costed 21.6s in run 01M550CAX per the prompt's own comment), but the brief's last bullet said *"Verification: prompt-lint literal hygiene and gate green via the deterministic tester step"* — naming the tier invited the pre-run. 3 of the implementer's 6 shell calls were this duplication, in the run's longest, costliest stage (78.8s, $0.081).
- **Change:** in `.fabro/workflows/loop/prompts/planner.md`, the "Envelope note for the brief" and step-7 guidance: drop `(lint-nu, prompt-lint, validate)` from the verification-criteria phrasing — the bullet must say only "gate green via the deterministic tester step (the tester records the lint tiers; never pre-run them)".
- **Expected effect:** kills the recurring double battery (~3 whole-repo calls + ~7KB of duplicated transcripts in evidence per run; directly attacks the largest stage).
- **Seed:** none open covers it — *new-seed justification: fabro-89bc (closed) landed the implementer-side rule, but no open seed covers the planner brief template still naming lint tiers, which is the leak this run proved.*

### 2. Make `fabro_run_search` reject unknown statuses with the valid enum
- **What happened:** planner's first `fabro_run_search` call failed with `unknown run status 'awaiting_review'` (from run events, tool call `call_3f804c157cde4691868b20dd`); it retried unfiltered and got 20 runs. Loop planner.md step 5 says "open PRs and non-terminal runs" without enumerating valid values (the develop prompt does enumerate them).
- **Change:** error message in the tool names the valid `RunStatusKind` values (seed's scope); optionally mirror the enum in `.fabro/workflows/loop/prompts/planner.md` step 5.
- **Expected effect:** removes one errored call + retry turn per planner pass.
- **Seed: fabro-1cde** (open, unassigned) — exact match.

### 3. Fix the run-title 100-char rejection path
- **What happened:** planner's first `fabro_run_set_title` (112-char title) was rejected — `run title must be at most 100 characters` (from run events, `call_271c8c4492bc445d87904a43`); second attempt (88 chars) succeeded. The planner.md RUN TITLE paragraph never mentions the cap.
- **Change:** either raise the cap (engine side, per the seed) or add "keep the title under 100 characters" to the RUN TITLE line in `.fabro/workflows/loop/prompts/planner.md`.
- **Expected effect:** removes one guaranteed-failing call per run; titles stop being truncated mid-thought on retry.
- **Seed: fabro-b529** (open) — covers the cap being too tight; this run is fresh evidence for it.

### 4. Replace the `seeds ready` firehose with a top-N view
- **What happened:** the planner's first tracker call dumped all 57 ready rows — 8,271 bytes retained, `stdout_truncated: true` (from run events) — of which it used the top ~5.
- **Change:** implement the seed: top-N `seeds ready` view (limit ~10) with fall-through paging, wired into `.fabro/workflows/loop/prompts/planner.md` step 1's command table.
- **Expected effect:** ~8KB less planner context per pass, faster candidate triage, fewer wasted input tokens (planner input was 27,919 tok).
- **Seed: fabro-c3b4** (open) — exact match.

### 5. Bound the in-flight exclusion check (self-inclusion + missing seed ids)
- **What happened:** the successful `fabro_run_search` returned **this run itself** as row 1 (status `running`), and results carry only `goal_preview` — no claimed-seed field — so matching candidates to in-flight seeds means reverse-engineering goals. Both are exactly why the planner needed 2 calls on this tool.
- **Change:** implement both seeds: `created_since` window + explicit self-exclusion (fabro-6b58), and expose `claimed_seed_id` in the runs projection (fabro-9372).
- **Expected effect:** no false self-match, mechanical candidate↔run matching, fewer planner tool turns.
- **Seeds: fabro-6b58 and fabro-9372** (both open) — exact matches.

### 6. Diff-scope the loop gate's lint output
- **What happened:** tester@1 output was 8,757 bytes for a one-line prompt edit: 6 synthetic-fixture-id note lines (~70 ids), 16 date-pin warnings, 19 routing-named schema warnings — all pre-existing and unrelated to the diff (from tester output in the run projection). This all rides into the reviewer's preamble.
- **Change:** in `.fabro/workflows/loop/scripts/loop-gate.nu`, filter prompt-lint notes/warnings to files present in the run diff; emit one summary line for the pre-existing remainder ("35 pre-existing warnings, 0 in-diff").
- **Expected effect:** tester output ~8.8KB → ~2KB; a real gate-red signal stops being buried; cheaper reviewer context.
- **Seed:** none open — *new-seed justification: searched "loop-gate noise", "noise", "prompt-lint", "fixture" — fabro-0d16 covers only the marker name drift and fabro-8dd8 only a missing positive line; nothing covers diff-scoping gate lint output.*

### 7. Deduplicate what the reviewer reads (tester output vs evidence recorded checks)
- **What happened:** the prompt-lint transcript appears **twice** in the reviewer's context — once as the tester stage section (tester is *not* in reviewer's `x.preamble_stages_ignore`), once re-embedded verbatim in evidence's "recorded checks" (17.1KB capture, from evidence output). Reviewer input: 26,172 tokens, $0.045.
- **Change:** two-part, either suffices: (a) in `.fabro/workflows/develop/scripts/evidence.nu`, collapse recorded checks whose commands the tester re-runs into `command → exit N` one-liners; or (b) in `.fabro/workflows/loop/workflow.fabro`, add `tester` to the reviewer node's `x.preamble_stages_ignore` (evidence already carries the gate verdict).
- **Expected effect:** ~10KB less reviewer context per review — lower cost and lower misread risk when the diff is small.
- **Seed:** none open — *new-seed justification: searched "evidence duplicate", "verification" — closed seeds f759/d89a added the per-criterion capture, but none covers deduplicating it against the tester's own output.*

### 8. File the date-pin revisor sweep the implementer already journaled
- **What happened:** the implementer's journal (from implementer stage output) flags "35 pre-existing warnings… date-pin warnings are drifting upward and may deserve a revisor sweep" — 16 stale date pins across develop/loop/revisor/conductor, visible in both tester and evidence output.
- **Change:** file the sweep seed (revisor lane): re-validate or retire every date pin older than 45 days across the four workflows.
- **Expected effect:** 16 of 35 warnings gone; load-bearing pins re-verified instead of silently rotting.
- **Seed:** none — *new-seed justification: searched "date pin", "date-pin", "prompt-lint", "sweep" — no open seed covers refreshing stale date pins; the journal observation has no seed yet.*

### 9. Heartbeat notification between terminal Slack events
- **What happened:** Slack config fires only on `run.completed`/`run.failed`; the 64s planner and 79s implementer stages (from stage telemetry) were completely silent for the run's 3.5-minute middle.
- **Change:** implement the seed's long-stage heartbeat (e.g., stage_complete > 60s posts to `#dev-fabro`).
- **Expected effect:** live visibility into which stage a scheduled run is in, before anything goes wrong.
- **Seed: fabro-b769** (open) — exact match.

---

**Dedup searches run:** `date pin`, `date-pin`, `prompt-lint`, `loop-gate noise`, `noise`, `evidence duplicate`, `implementer verification battery`, `run_search`, `battery`, `sweep`, `title`, `fixture`, `verification`, `pre-gate`. Items 1, 6, 7, 8 carry explicit new-seed justifications; all others cite an existing open seed id.
