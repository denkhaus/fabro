# Revision — run 01M5CR2B9DKQ0ZRXKD3DZSPMP0

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M5CR2B9DKQ0ZRXKD3DZSPMP0.md
- seeds filed: none — zero same-pass credit (ADR-0022); 4 surviving findings journaled below (3 new overflows + 1 overflow-dup link)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M5CR2B9DKQ0ZRXKD3DZSPMP0, workflow version absent, commit bb57b5fa36b26da20817cd2de0da0b2d899de430
- revised_at_commit: bb57b5fa36b26da20817cd2de0da0b2d899de430 (ADR-0015: engine drift signal for later judgement)

## Findings

Closeout-race re-check at filing time: `{"outcome":"ok","run":"01M5CR2B9DKQ0ZRXKD3DZSPMP0","count":0,"matching":[]}` — no closeout seeds cite the target; no duplicates dropped. Findings 2,3,4,5,9 from the review matched existing open seeds (fabro-1cde, fabro-b529, fabro-c3b4, fabro-6b58, fabro-9372, fabro-b769) — no action, already tracked.

**1. Diff-scope the loop gate's lint output** — overflow

- overflow: Diff-scope the loop gate's lint output — in `.fabro/workflows/loop/scripts/loop-gate.nu`, filter prompt-lint notes/warnings to files present in the run diff and emit one summary line for the pre-existing remainder ("35 pre-existing warnings, 0 in-diff"); effect: tester output ~8.8KB → ~2KB (this run: 8,757 bytes for a one-line prompt edit — 6 synthetic-fixture-id note lines, 16 date-pin warnings, 19 routing-named schema warnings, all pre-existing), so a real gate-red signal stops being buried and reviewer context gets cheaper. No open seed covers it (analyzer searched "loop-gate noise", "noise", "prompt-lint", "fixture"; fabro-0d16 = marker drift, fabro-8dd8 = missing positive line). Distinct from open overflow "Batch prompt-lint synthetic-id fixture notes to one summary line" (`01M5CPBDF5PNC8EGDK805SVW10.md`, prompt-lint.nu itself) — that collapses the note source, this filters at the gate by diff.

**2. Deduplicate what the reviewer reads (tester output vs evidence recorded checks)** — overflow

- overflow: Deduplicate what the reviewer reads — either (a) in `.fabro/workflows/develop/scripts/evidence.nu`, collapse recorded checks whose commands the tester re-runs into `command → exit N` one-liners, or (b) in `.fabro/workflows/loop/workflow.fabro`, add `tester` to the reviewer node's `x.preamble_stages_ignore` (evidence already carries the gate verdict); effect: ~10KB less reviewer context per review (this run: prompt-lint transcript appears twice — tester section plus verbatim re-embed in evidence's 17.1KB "recorded checks"; reviewer input 26,172 tokens, $0.045), lower cost and lower misread risk. No open seed covers it (closed f759/d89a added per-criterion capture only). Distinct from open overflow "Render very long diff lines readably in the evidence capture" (`01M5CN9EC11K39YCG4HCHDJ6NE.md`) — different mechanism (duplication vs line length).

**3. File the date-pin revisor sweep the implementer already journaled** — overflow

- overflow: Re-validate or retire every prompt-lint date pin older than 45 days across develop/loop/revisor/conductor workflows (revisor lane sweep); effect: 16 of 35 pre-existing warnings gone and load-bearing pins re-verified instead of silently rotting (this run: implementer journal flags "35 pre-existing warnings… date-pin warnings are drifting upward and may deserve a revisor sweep"; 16 stale pins visible in tester and evidence output). No open seed covers refreshing stale date pins (analyzer searched "date pin", "date-pin", "prompt-lint", "sweep"). Distinct from open overflow "Silence by-design prompt-lint warnings in the qualitygate" (`01M2XQJC3TWRMJ1128WRXRQYXH.md`) — that allowlists the nag in `qualitygate.nu`; this refreshes the pins themselves.

**Planner brief must stop naming lint tiers (double-battery leak)** — linked, not re-journaled

- overflow-dup: Scope implementer pre-gate checks to the diff, not the whole repo (open in `.fabro/revisions/01M550CAXV85JGT1XBPCDRE5DY.md`) — this run's fresh evidence for the same theme: the planner brief's verification bullet ("prompt-lint literal hygiene and gate green via the deterministic tester step") named the tiers and the implementer pre-ran `nu .fabro/scripts/prompt-lint.nu` (81 files), `just lint-nu` (67 scripts) verbatim before the tester re-ran them 0.4s later (3 of 6 implementer shell calls, in the run's longest stage 78.8s/$0.081); the open overflow's fix direction (`.fabro/workflows/loop/prompts/planner.md` envelope: leave whole-repo lint/battery tiers exclusively to the tester) is the same target — closed fabro-89bc landed only the implementer-side rule.
