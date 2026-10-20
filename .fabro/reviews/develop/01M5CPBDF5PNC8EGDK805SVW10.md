# Improve review — run 01M5CPBDF5PNC8EGDK805SVW10

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (281.8s, revisor pass — reason and cost in run detail)
- generated: 2026-10-20 13:14+0000 by revisor `fabro_ask`

---

All recommendations below are grounded in this run's own record (stage transcripts, gate output, journal observations, timings, cost from run events / `fabro_run_get`). Run economics for context: 281.8s wall, $0.27 total; **planner alone was 140.9s (50% of wall) and $0.164 (60% of cost)**; implementer 71.7s/$0.077; reviewer 18.2s/$0.034; tester 14.5s. Searches run for dedup: `sd ready firehose planner`, `in-flight check planner`, `gh CLI sandbox`, `prompt-lint warnings`, `implementer probes single-file`, `preflight legend`, `date pin older 45 days`, `routing-named schema warning`, `in_flight terminal run stale`, `check-transcript`, `preamble inline evidence truncation`, `date pins stale prompts`.

---

**1. Shrink the planner's candidate context to a top-N view — seed `fabro-c3b4` (open)**
- What happened: `seeds ready --assignee loop --limit 200` dumped **58 rows / 8,443 bytes** into planner context (event `execution 0/56`) for a lane that is hard-bound to one seed per run; the preflight JSON adds a **static ~2.5KB legend re-inlined every run** (visible in the planner's UserInput preamble, event `execution 0/49`). The planner still cost 60% of the run.
- Change: node `preflight` / `.fabro/workflows/develop/scripts/planner-preflight.nu` — emit only top-N candidates (N≈5) and move the static legend into `.fabro/workflows/loop/prompts/planner.md` once instead of `context_updates` per run; drop the `--limit 200` full firehose per fabro-c3b4.
- Effect: cuts planner input/context and reasoning turns on every loop run — the single biggest cost/latency lever this run exposes (planner = 50% wall, 60% cost).

**2. Port the single-file probe rule into the implementer prompts — new seed needed**
- What happened: the brief explicitly said "Implementer-stage probes are single-file only … do NOT run repo-wide lint or battery tiers" (planner response), yet the implementer's recorded checks were **whole-repo `nu .fabro/scripts/prompt-lint.nu` (81 files) and `just lint-nu` (67 scripts)** — and the tester re-ran both tiers 73s later (tester output shows both again). The rule currently lives only in `.fabro/workflows/loop/prompts/planner.md:42` (verified by grep); the implementer prompt never states it.
- Change: add the single-file probe rule (exact-string grep / `nu -c "source <script>"`) to the check-transcript section of BOTH `.fabro/workflows/loop/prompts/implementer.md` and `.fabro/workflows/develop/prompts/implementer.md`.
- Effect: kills the double battery run (previously measured at 21.6s/run, run 01M550CAX; this run's implementer tool time 8.6s + duplicate tester tiers) and shrinks evidence captures.
- New-seed justification: `fabro-89bc` (same fix) is **closed** and this run proves the pattern recurred because the rule was only enforced via the planner brief, not the implementer prompt.

**3. Triage prompt-lint's 35 warnings and 70 synthetic-id notes — new seed needed**
- What happened: gate output (8.8KB) carried **18 stale date-pin warnings** (incl. `loop-gate.nu` pinned 2026-04-14), **18 routing-named warnings on the 4 schemas that are *intentionally* routing schemas**, and 6 fixture notes enumerating 70 synthetic ids — and this block appears **three times** in the run record (implementer checks, tester output, evidence capture), pushing `evidence` output to **16,928 bytes — above the reviewer's `x.preamble_inline_max_kb=16` (16,384B)**, i.e. the fabro-31f7 truncation class recurring at the byte cap.
- Change: `.fabro/scripts/prompt-lint.nu` — mark routing schemas as intentionally routing (suppress), batch synthetic-id fixture notes to one summary line, and resolve or waive the 18 date pins.
- Effect: gate output becomes signal; evidence drops below the reviewer inline cap so the reviewer reads captures whole instead of a blob-ref tail.
- New-seed justification: searches for date-pin/routing-named/prompt-lint-warning seeds returned zero matches; `fabro-0d16` covers only the marker-name drift.

**4. Make preflight resolve run terminality before flagging `in_flight` — extend seed `fabro-6b58` (open)**
- What happened: preflight at 13:00:41 marked fabro-16a7 and fabro-8932 `in_flight` under run `01M5CN9EC11K39YCG4HCHDJ6NE`, but the planner's `fabro_run_search` at 13:00:49 showed that run **succeeded at 12:49:56** — terminal 11 minutes earlier. The planner burned a tool call + reasoning compensating (its journal observation #3 admits this).
- Change: `.fabro/workflows/develop/scripts/planner-preflight.nu` — check run status before emitting `in_flight: true`; a `created_since` window alone won't fix it (the stale run was created only 19 min before this one).
- Effect: removes a guaranteed per-run compensating tool call + reasoning when a prior loop run recently finished; seed fabro-6b58 is the right vehicle (its window bound plus this terminality filter).

**5. Decide the run-level fabro-tools question for the degraded open-PR arm — seed `fabro-d62c` (open, needs-user)**
- What happened: planner journal observation #1: "**gh CLI absent in this sandbox (exit 127) — open-PR arm of the in-flight check ran degraded**; compensated via fabro_run_search."
- Change: this is the user-decision seed fabro-d62c — enable run-level fabro tools (or add `gh` to `ghcr.io/denkhaus/fabro-toolchain`) so the open-PR arm stops running degraded every loop run.
- Effect: the in-flight exclusion check runs on primary data instead of a documented workaround; removes the recurring degraded-mode journal entry.

**6. Enforce the transient-failure rule mechanically in the wrapper — new seed needed**
- What happened: this run landed fabro-16a7 as **prompt prose only**, and the implementer's own journal flags the gap: "prompts are advisory — nothing mechanical blocks a PASS line whose only record is a transient non-zero exit; a future seed could add a lint in `.fabro/scripts/check-transcript.nu` that flags consecutive same-command records with differing exit codes."
- Change: `.fabro/workflows/develop/scripts/check-transcript.nu` — flag consecutive same-command records with differing exit codes (transient fail → later PASS) so an unre-ran transient failure can't hide behind a PASS line.
- Effect: the exact failure class this run patched becomes structurally impossible, rather than advisorily discouraged, on both lanes.
- New-seed justification: `fabro-e32c` (only check-transcript seed found) is closed and covers quoting mangling, not exit-code-sequence enforcement; no open seed covers it.

**7. Normalize the loop implementer's check-transcript mega-paragraph — new seed needed**
- What happened: the reviewer's minor observation: the same new rule landed as a clean 5-line `TRANSIENT FAILURES:` block in develop's prompt but was **inlined into one ~1,900-character single-line paragraph** in `.fabro/workflows/loop/prompts/implementer.md` (line 48) — consistent with that file's structure today, but every future one-unit prompt edit will keep drifting and produce unreviewable single-line diffs.
- Change: restructure that paragraph in `.fabro/workflows/loop/prompts/implementer.md` into labeled blocks mirroring develop's (`$?` assertion rule / TRANSIENT FAILURES / NESTED-SHELL PROBES).
- Effect: future one-unit prompt edits (the loop lane's most common seed type — 40+ prompt-revision seeds are queued) diff cleanly and review faster.
- New-seed justification: no existing seed covers loop-prompt structural parity (searches on prompt/paragraph/parity themes returned none); it's the reviewer's own flagged residual from this run.

Not recommendable from this run: the reviewer's 0 cache-read (engine-level session behavior, no file-level lever here) and the one errored `fabro_run_set_title` retry (title hit the 100-char limit; too small to seed on its own, but folding a "≤100 chars" note into the planner's ADR-0030 title paragraph would cheaply remove the retry).
