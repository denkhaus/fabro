# Revision — run 01M5JRAP1F67RZE674TBRPGPY8

- status reviewed: succeeded
- review: `.fabro/reviews/develop/01M5JRAP1F67RZE674TBRPGPY8.md`
- items filed: none — tracker write surface unavailable this pass (`fabro_work_create` fails with "no work project is bound to target repository denkhaus/fabro"; no `sd` CLI; commit scope gate forbids `.seeds/issues.jsonl`); both surviving findings journaled as overflow below for the next pass
- balance: 0 non-exempt items filed / 0 — no credit this pass
- basis: run 01M5JRAP1F67RZE674TBRPGPY8, workflow version absent, commit 1bae04ff4cc19224484d1d3c00a9ea3fafec6909
- revised_at_commit: 1bae04ff4cc19224484d1d3c00a9ea3fafec6909 (ADR-0015: engine drift signal for later judgement)

## Findings

**1. Resolve workflow-relative shorthand paths in claim-check/preflight** — overflow
- overflow: Resolve workflow-relative shorthand paths in claim-check/preflight — in `.fabro/workflows/develop/scripts/claim-check.nu` and `planner-preflight.nu`, resolve `<lane>/<file>` citations against `.fabro/workflows/<lane>/` before flagging/harvesting; effect: preflight's 3 `missing_file` false positives (`revisor/workflow.fabro`, `develop/workflow.fabro`, `loop/workflow.fabro` on 3 of 5 candidates this run) stop forcing hand adjudication, and claim_check's spec-paths arm stops reporting `spec-paths []` / "no range to scan — arm skipped (fail-open)" — deletes the hand-verification pass from both LLM stages every run. Filing-time dedupe (degraded, seeds grep — tracker query dead): closed fabro-48f3/fabro-bf3c/fabro-513e are gate-side spec-paths/recipe seeds, closed fabro-7611 fixed the crate-relative analogue only; distinct from open overflows "Planner-preflight anchor check: suffix-resolve bare filenames" (`01M2XYE58D9210W751TTJT2P64.md`, bare-filename suffix match) and "kill the tester spec-paths bounce" (`01M5CVG6XT9ABZFTMGY45QGCKT.md`, context emission).

**2. Loop reviewer node cannot read the evidence blob** — overflow (needs-user per ADR-0019 — file under the needs-user exemption, never against balance, when the tracker binding lands)
- overflow: Loop reviewer node cannot read the evidence blob — capability-affecting per ADR-0019: arm (a) grant the loop reviewer node a read-only blob-read tool via `x.fabro_tools` in `.fabro/workflows/loop/workflow.fabro` (adds a tool to an agent-reachable allowlist — implementation awaits explicit user approval), or arm (b) the non-capability alternative: `evidence.nu --lane loop` inlines a capped plain-text capture (the develop-lane analogue is closed fabro-8cef); effect: the reviewer judges the captured evidence instead of falling back to the working tree — this run's capture was a 100,765-byte `blob://sha256/7d978f0c…` the reviewer could not read ("no fabro_blob tool — the allow-list admits read_file/grep/glob only"), so the 2m-timeout evidence stage was dead weight and approval drifted from what was captured. Filing-time dedupe (degraded, seeds grep): open fabro-eed2 (prompt-doc), fabro-7613 (engine auto-stamp), fabro-9a43 (read_file offset/limit phrase checks) and closed fabro-9837/fabro-d774/fabro-a0fe/fabro-8cef (develop-lane paging/rendering) — none names the loop reviewer node's missing read grant.

3. Diff-scope the loop gate's prompt-lint output (8.5KB tester output, 35 pre-existing warnings) — duplicate_of: fabro-a827 (open, @loop)
4. Deduplicate the reviewer's tester-vs-evidence reading (46,903-token reviewer input) — duplicate_of: fabro-17fb (open, @loop)
5. Ship the nushell-scripts skill for the implementer — duplicate_of: fabro-d19e (open, @loop)
6. Run-title 100-char cap (4 planner calls, 3 rejected this run) — duplicate_of: fabro-b529 (open; cross-lane proof)
7. Sweep stale revisor date pins (4 warnings >45 days, touched unswept) — duplicate_of: fabro-801e (open, @loop)
8. Long-stage heartbeat (15-minute implementer black box) — duplicate_of: fabro-b769 (open, @loop)
9. Top-N the planner's ready view (`seeds ready --limit 200` → 9.2KB into planner context) — duplicate_of: fabro-c3b4 (open, @loop)

## Notes

- Filing-time closeout re-check (degraded — tracker query dead, seeds grep): exactly four items cite this run id (fabro-85f6, fabro-9dd6, fabro-3ec9, fabro-1923, all the run's own closeout filings); none covers findings 1–2. No race.
- Tracker outage is the fourth consecutive revisor stage hitting "no work project is bound to target repository denkhaus/fabro" (analyze pass 1, file pass 1, analyze pass 2, file pass 2). Until the binding lands, every filing pass degrades to overflow journaling.
