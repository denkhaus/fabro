# Improve review — run 01M5JRAP1F67RZE674TBRPGPY8

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (19.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 03:08+0200 by revisor `fabro_ask`

---

All evidence is gathered. Here are the recommendations, grounded in this run (loop workflow, seed fabro-3d9a, succeeded 21:30–21:50, 19.9 min wall, $2.05 total cost, 13 files +248/−350, PR #466 — from run events).

**Run economics that drive the ordering** (from the run conclusion): implementer = 924.7s wall / 891.0s inference vs only 32.3s tool time, $1.80 of the $2.05 total (88%); reviewer = 83.2s / $0.14; planner = 96.1s / $0.11 with 3 failed `fabro_run_set_title` calls; tester green in 54.4s but emitting 8.5KB of mostly diff-unrelated warnings.

---

**1. Make the evidence capture readable by the reviewer — highest correctness risk in this run.**
- Evidence: reviewer journal painpoint — evidence arrived only as `blob://sha256/7d978f0c…` (100,765 bytes) but the reviewer node's allow-list (`read_file,grep,glob`, `fs_write=""` in `.fabro/workflows/loop/workflow.fabro`) has no blob reader; the review "fell back to verifying against the working tree" with 14 re-derived tool calls instead of judging the capture.
- Change: in `.fabro/workflows/develop/scripts/evidence.nu` (shared, `--lane loop`) emit an inline, capped digest (integrity header + per-file heads) under the reviewer's 16KB `x.preamble_inline_max_kb`, keeping the blob as appendix — or grant the reviewer node a blob-read tool.
- Expected effect: the reviewer judges captured evidence instead of re-deriving it — shorter reviews and no risk of reviewing the wrong tree state on retries.
- **New-seed justification:** no existing seed covers the evidence-stage output blob being unreadable to the reviewer node (searched `blob`, `blob reviewer`, `evidence capture reviewer inline`; closed predecessors fabro-31f7/8cef/9837/d774 addressed preamble paging, not this) — and this run's closeout filed only fabro-85f6/fabro-9dd6 from the reviewer journal, so this painpoint went unfiled.

**2. Cut implementer LLM round-trips by allowing batched read-only probes — biggest cost/time lever.**
- Evidence: 61 shell + 19 edit_file calls across 84 messages; inference 891s vs 32s tool time — nearly all wall time is turn overhead. The planner prompt mandates "run each probe as its OWN shell call" (seen in the planner's prompt transcript, from run events).
- Change: in `.fabro/workflows/loop/prompts/planner.md` (step 8) and `prompts/implementer.md` (verification section), permit batching independent read-only probes per shell call with per-segment markers (`echo "== probe <name>"; <cmd>; echo "exit=$?"`) — markers preserve the failure visibility the `;`-chaining ban was protecting.
- Expected effect: roughly halve implementer turns (est. 61→~25 probe calls), directly attacking the stage that consumes 88% of run cost and 77% of wall time.
- **New-seed justification:** no seed covers probe batching (searched `probe own shell call batch` → 0; fabro-b6f9/fabro-8d4c cover stderr discipline and heading greps only).

**3. Diff-scope the loop gate's lint output — seed fabro-a827 (open, @loop).**
- Evidence: tester output 8,514 bytes, 35 prompt-lint warnings nearly all about files outside the 13-file diff (18 date-pin warnings across conductor/develop/loop/revisor, routing-named schema warnings on 4 schemas, synthetic-id notes) — and this output flows into the reviewer's 16KB preamble budget.
- Change: in `.fabro/workflows/loop/scripts/loop-gate.nu`, filter prompt-lint warnings to diff-touched files; print counts only for the rest (fabro-801e separately owns retiring the stale revisor date pins).
- Expected effect: gate output becomes actionable signal; frees reviewer preamble space for evidence.

**4. Fail the claim when the seed body names zero spec paths — deterministic coverage regressed to by-hand this run.**
- Evidence: claim_check printed `spec-paths []`; tester logged "seed-spec-paths [fabro-3d9a] — spec names no loop-asset paths, tier idle"; implementer journal: "the seed-spec tier enforces nothing here — the revisor-path coverage check was done by hand against git diff."
- Change: in `.fabro/workflows/develop/scripts/claim-check.nu` (shared), treat `spec-paths == []` on a claimed task seed as a failed claim — bounce to the planner to fix the body or add an explicit waiver marker.
- Expected effect: the one-unit coverage guarantee becomes deterministic again instead of depending on implementer diligence.
- **New-seed justification:** no open seed covers the tier's silent idle on path-less bodies (searched `spec-paths`, `spec names no loop-asset`; fabro-48f3/fabro-bf3c are closed and cover different facets).

**5. Resolve workflow-relative shorthand anchors before flagging `missing_file`.**
- Evidence: this run's preflight flagged false `missing_file` anchors on three of five candidates (fabro-3d9a `revisor/workflow.fabro`, fabro-d48f `develop/workflow.fabro` + `develop/planner.md`, fabro-4653 `loop/workflow.fabro`); the planner journaled it as "a shorthand-path false positive," and claim_check repeated two of the flags.
- Change: in `.fabro/workflows/develop/scripts/planner-preflight.nu` and `claim-check.nu`, try repo-root, then `.fabro/workflows/<lane>/` and its `prompts/` prefix before flagging.
- Expected effect: removes a recurring manual adjudication from every loop planning pass and shrinks preflight noise.
- **New-seed justification:** fabro-7611 (closed) fixed only crate-relative anchors; no open seed covers workflow-relative shorthand resolution (searched `shorthand anchor preflight`, `anchor missing_file`).

**6. Extend lint-nu's bare-paren check to comma-less prose parens.**
- Evidence: implementer painpoint — the new `overflow-ledger.nu` bad-key message `(or a legacy …)` executed command `or` (exit 1 instead of 2), caught only by the fixture negative case; "parsed green while failing at runtime." From workspace file `scripts/lint-nu.nu`: `interpolated-regex-check` flags only `(?P<`/`(?<`/`(?:` groups and `paren-comma-bare` only comma-list parens — this case slipped both.
- Change: in `scripts/lint-nu.nu` `paren-comma-bare`, also flag unescaped `(`-groups inside `$'…'`/`$"…"` spans that start with a bare word + space (prose-shaped).
- Expected effect: this runtime-only bug class dies at gate time instead of in production error paths.
- **New-seed justification:** no open seed covers the static-lint extension (searched `interpolated paren lint-nu`, `paren`; fabro-7762 is closed and was point-of-use warnings only).

**7. Long-stage heartbeat notification — seed fabro-b769 (open, @loop).**
- Evidence: implementer ran silently for 15m25s; run notifications are terminal-only (`run.completed`/`run.failed` → Slack, from the run spec) and `stall_timeout=33m`, so a wedged stage is invisible for up to half an hour.
- Change: per fabro-b769, engine heartbeat notification for stages exceeding N minutes.
- Expected effect: humans see progress/intervene long before the stall breaker fires.

**8. Note the 100-char title cap in the planner prompt — seed fabro-b529 (open, revision/engine).**
- Evidence: planner burned 4 `fabro_run_set_title` calls with 3 errors ("had to trim twice" — planner journal, from run events).
- Change: one line in `.fabro/workflows/loop/prompts/planner.md`'s RUN TITLE paragraph: "≤100 chars, front-load the demand" (the seed owns the engine-side cap widening).
- Expected effect: eliminates 3 wasted tool calls + retries every claiming run.

**9. Require English responses in the loop planner too — seed fabro-eaa0 (open, @loop).**
- Evidence: both agent responses this run opened in German ("Gefunden und geclaimt…", "Die Revisor-Unit ist vollständig…") while titles, briefs, and the PR body are English.
- Change: apply fabro-eaa0's rule in `.fabro/workflows/loop/prompts/planner.md` (the seed names the implementer prompt; extend it).
- Expected effect: consistent downstream artifacts and no translation friction for the PR-merger model.

Dedupe searches run (all via the seeds tracker): `fabro-85f6`, `fabro-9dd6`, `fabro-3ec9`, `fabro-1923`, `title 100`, `anchor missing_file`, `spec-paths`, `interpolated paren lint-nu`, `paren`, `heartbeat`, `blob`, `shorthand anchor preflight`, `seeds ready firehose`, `English responses`, `spec names no loop-asset`, `probe own shell call batch`, `evidence capture reviewer inline`. Not inspected: the four new seeds' full bodies (titles only are exposed by search), and the evidence blob contents itself (blob store not reachable from here).
