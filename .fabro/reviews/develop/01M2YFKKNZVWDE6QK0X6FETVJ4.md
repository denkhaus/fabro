# Improve review — run 01M2YFKKNZVWDE6QK0X6FETVJ4

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-20 04:10+0000 by revisor `fabro_ask`

---

All evidence gathered — from run events (stage transcripts, timings, usage), the worker warn/error log, the tester gate output, and the tracker (`.seeds/issues.jsonl`, read via workspace file tools). Run context for scale: seed **fabro-8c8a** (one-sentence prompt edit), 10 stages, ~2.9 min wall, **$0.198 total**, planner alone $0.115 (58%). Recommendations ordered by expected impact.

---

## 1. Make the preflight `in_flight` marker trust run/PR terminality — unblocks 2 stranded seeds and kills the planner's adjudication lap
**Seed: fabro-ab93 (open, P1)** — change `.fabro/workflows/develop/scripts/planner-preflight.nu`'s in-flight predicate (mark `in_flight` only when the run is non-terminal OR its PR is open/unknown-with-branch).
**What happened here (run events):** the preflight marked **fabro-af22 and fabro-b03f** `in_flight: true` via run `01M2YDXWZQC5XEFRG7HTMNTQCB` — which is **terminal-failed** (completed 03:41, no PR, confirmed in the planner's own `fabro_runs_list` result). The planner then burned a visible multi-turn deliberation on exactly this contradiction (its reasoning trace, seq 52: *"run … is FAILED, no PR. But preflight marked them… Hmm, tricky… So skip"*) before skipping both seeds anyway and journaling *"a later run may need to adjudicate reclaiming them."* That adjudication sits inside the planner's 38.9 s inference / 52.9 s stage — the single largest cost block of the run — and **fabro-af22 is itself an open P1** (skill-expansion crash class) now parked behind a stale marker.
**Expected effect:** planner skips the ~20–30 s adjudication window on every affected pass, and two open seeds (one P1) return to the candidate pool instead of waiting for the 6 h tracker-guard requeue.

## 2. Bound the planner's `fabro_runs_list` call — 13.4 s and ~100 KB for a fact the preflight already had
**Seed: fabro-6b58 (open)** — change `.fabro/workflows/develop/prompts/planner.md` step 4 to pass `created_since ≈ 48h` plus explicit self-run exclusion (its exact ask); engine complement is open **fabro-9372** (`current_seed_id` in the projection).
**What happened here (run events):** the planner's single `fabro_runs_list` call ran 04:01:46.4→04:01:59.8 (**13.4 s — 97% of the stage's 13.8 s tool time**) and returned **107 runs**; the payload so dominated the transcript that my own event read truncated it (fabro-6b58's basis measured 16 runs ≈ 14.5 KB — this was ~7× that). The planner needed only open-PR/non-terminal entries; everything else was terminal noise it re-scanned (and the preflight's branch-scan arm had already covered the in-flight candidates).
**Expected effect:** tool output drops from ~100 KB to ≤2 KB and ~10 s off planner wall on **every** pass — this call happens in all 107+ runs of this line.

## 3. Briefs must annotate quoted insert-text (case/format) — the "verbatim lowercase 'a'" cost a re-edit and shipped broken prose
**New-seed justification:** no existing seed covers quote-insertion handling in briefs — open fabro-7773 lints seed-spec contradictions against pinned *behavior*, closed fabro-cf76 covered heading/anchor verification; neither covers quoted-insert grammar, and the defect is now live in the tree.
**What happened here (run events + journal):** the brief demanded the sentence *"verbatim"*, but the quote starts lowercase after a period. The implementer journaled: *"I initially capitalized it and had to re-edit to satisfy the verbatim criterion"* — a wasted edit/verify cycle in a 42 s stage — and the merged `planner.md` line 30 now reads *"…fewest blockers. a needs-user seed is claimable…"*, grammatically broken, re-read by every future planner pass. The reviewer flagged it (*"a future copyedit could capitalize after the period"*) but the note evaporated at approval — the exact channel gap of open **fabro-89dd** (`review_notes` for non-blocking findings).
**Concrete change:** add one rule to `planner.md` step 7 (contradiction check): when a brief quotes text for verbatim insertion, normalize the quote or annotate the exact-bytes requirement including capitalization; same seed carries the one-line copyedit (capitalize the 'a' at `planner.md` line 30); route the reviewer's evaporated note through fabro-89dd's channel.
**Expected effect:** no re-edit cycles on quoted-insert seeds, and no grammar defects landing in prompts that every subsequent run re-reads as instructions.

## 4. Stop by-design warns/errors firing on green runs — 9 noisy lines in a 100%-green run
**Seed: fabro-8275 (open)** — extend its demotion (absent-on-first-visit → info) beyond the preamble lint to the other two by-design emission points observed here.
**What happened here (worker log, via `fabro_run_logs`):** a fully green run emitted 9 warn+ lines: (a) `preamble_allow_keys entry absent … key=output.gatebounce` — by design, gatebounce never runs on a green pass (fabro-8275's exact class, now under the key renamed by closed fabro-7028); (b) `context_update_dropped: output.planner` — by design, the engine dedups the response into `response.planner` and drops the duplicate every planner pass; (c) **6× ERROR** `File "/workspace/fabro/.codex/instructions.md" was not found` — two per agent session × 3 sessions, an optional file probed at every session init. (New-seed note for (c) if kept separate: no existing seed covers the session-init optional-file read; fabro-8275 is scoped to the preamble warn.)
**Concrete change:** in `lib/components/fabro-workflow` fidelity/notice paths, demote first-visit-absent and dedup-drop to info; probe-or-demote the `.codex/instructions.md` read.
**Expected effect:** green runs go to ~zero warn lines, so the one real fidelity regression (the class closed fabro-0a4c/fabro-c42f chased) is actually visible.

## 5. Adjudicate the gate's own recurring warnings — the date-pin question nobody answers
**New-seed justification:** no open seed covers prompt-lint warning adjudication — closed fabro-a211 only documented the routing-field contract; the toolchain date-pin warning has no carrier.
**What happened here (tester stage output):** the gate printed `warn: project-facts.md: date pin '2026-04-14' older than 45 days — still load-bearing?` (the pin is now ~5 months old, and it governs the fmt/clippy commands every Rust seed relies on) plus **10** `routing-named property` warnings across `planner-output.schema.json` and `develop-output.schema.json` — where routing-named fields are *intentional*. Summary line: "prompt-lint: ok — 43 files, 11 warnings"; the reviewer is instructed to treat the gate as opaque/green, so nobody ever answers the question the gate itself asks.
**Concrete change:** in `.fabro/scripts/prompt-lint.nu`, allowlist intentionally routing-named schemas (or demote to info) and make the date-pin check decision-backed (a reviewed-on date or an escalation, not an eternal warn).
**Expected effect:** 11 recurring warnings either disappear or become actionable; the one surviving warning class (toolchain staleness) finally gets a owner decision instead of wallpaper.

---

**Noted non-issue:** the PR-content `structured generation failed; retrying once without strict JSON` warn is closed fabro-cd27's designed retry path working (first glm-4.7 attempt non-JSON, retry succeeded, PR #310 published, auto-merge enabled) — no action warranted.

**Sources:** stage timings/usage and tool-call transcripts from run events (`fabro_run_events`, planner seq 34–70, implementer seq 83–133); warn/error lines from the persisted worker log (`fabro_run_logs`); gate warnings from the tester stage output; seed statuses from `.seeds/issues.jsonl` read via workspace file tools. What I could not inspect: the full untruncated 107-run `fabro_runs_list` payload (size estimated from fabro-6b58's measured per-run bytes) and PR #310's final merged state (PR event stream not in run scope).
