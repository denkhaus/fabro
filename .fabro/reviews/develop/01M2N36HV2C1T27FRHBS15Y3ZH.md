# Improve review — run 01M2N36HV2C1T27FRHBS15Y3ZH

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (13.1 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 12:49+0000 by revisor `fabro_ask`

---

Recommendations for the develop workflow, grounded in run `01M2N36HV2C1T27FRHBS15Y3ZH` (seed fabro-562a, PR #179 — clean first-pass cycle, 0 retries, 750.6 s wall, $0.910 total). Sources: run events/stage projections, stage journals, worker log, and `.seeds/issues.jsonl` (open/closed status verified). Ordered by expected impact.

## 1. Reviewer blob detour on churn-only evidence captures — seed **fabro-cf3e** (open; amend target)
**What happened:** the evidence capture was 34,840 bytes (seed-work = 0 files, loop-churn = 9 files +511/−1); the reviewer's `preamble_inline_max_kb=16` demoted it to a blob ref with a 3-line preview, forcing the review's only tool call (1 `read_file`, 45.1 s stage). The reviewer journaled it verbatim: "Evidence arrived only as a 35 KB blob ref… raise the evidence inline budget for churn-only seeds where the loop-work diff is the review scope."
**Change:** `.fabro/workflows/develop/workflow.fabro`, reviewer node — fabro-cf3e proposes 16→32, but **32 < 35.1 KB measured here**; set it to 40 (or compose with fabro-cf3e + dropping the duplicated seed-spec quote via open fabro-c1bb).
**Effect:** capture renders inline; the per-review tool detour and the unread-blob → "Verification blocked" rejection class disappear entirely.

## 2. Implementer error turns are pure inference waste — seeds **fabro-b09c** (open) + one new seed
**What happened:** implementer: 621.8 s inference vs **6.4 s tool time** (99%), $0.764 = 84% of run cost, 56 shell calls with **9 errors**. The worker log shows 29 `WARN … provider protocol does not support the tool result error flag` lines (12:38–12:42) — errored calls surface as stdout text the model re-interprets, and every retry is a full-context inference turn. The documented error class: `nu -c "…"` snippets losing `$`-variables to bash double-quote interpolation (implementer journal painpoint; recorded as mx-e606d5/mx-d44874 but nothing enforces it).
**Change:** (a) fabro-b09c — carry tool-result errors through the provider protocol instead of stdout text-sniffing; (b) new seed: extend `.fabro/scripts/prompt-lint.nu` to extract fenced shell commands from `.fabro/workflows/**/prompts/*.md` and flag double-quoted `nu -c` blocks containing `$`. *New-seed justification: fabro-f18a validates only JSON tool-call examples and explicitly skips shell examples; fabro-b6f9/fabro-8d4c govern planner probe commands, not prompt-content snippets.*
**Effect:** fewer failed-call/retry turns — the only lever that directly cuts the 621.8 s / $0.764 implementer dominance on config-only seeds.

## 3. Planner over-fetches the tracker — seed **fabro-66bc** (open)
**What happened:** `sd ready --assignee fabro --limit 200` returned 200 seeds / 27,938 bytes (event seq 30, stdout truncated in projection); the planner then picked the **first line** (fabro-562a, High priority) anyway.
**Change:** `.fabro/workflows/develop/prompts/planner.md` command table — top-N priority-sorted `sd ready` (e.g. 20), per fabro-66bc.
**Effect:** ~25 KB less planner context every run — cheaper turns, no truncation of the candidate list the planner actually reads.

## 4. `fabro validate` unavailable in the sandbox — **new seed**
**What happened:** implementer observation: "fabro binary absent — `fabro validate architect` could not run; parse-level structural checks (attribute names cross-checked against `lib/foundation/fabro-types/src/graph.rs`) stood in." That hand-rolled verification is inference-only burn inside the 621.8 s, and weaker than a real graph parse.
**Change:** bake a validate-capable `fabro` CLI into `.fabro/Dockerfile.toolchain` (local graph/TOML validation only — no credential, so no ADR-0019 capability delta). *New-seed justification: open fabro-0586 only documents which binaries exist in PROJECT_FACTS; no open seed adds the CLI to the image.*
**Effect:** workflow-authoring seeds verify with one deterministic command instead of minutes of ad-hoc structural reasoning; real syntax errors caught pre-gate.

## 5. Pipeline-progress headers still miscount — seed **fabro-9e8b** (open; this run is fresh counter-evidence)
**What happened:** the implementer stage prompt (seq 71) read "Pipeline progress: 0 of 7 stages completed" **after** planner completed; the reviewer prompt (seq 424) read "2 of 7" with start+planner+implementer+tester+evidence already done.
**Change:** engine progress projection surface in `lib/` (as named by closed fabro-45bf) — re-verify/fix the post-#175 counting; add this run's seq 71/424 as regression evidence to fabro-9e8b.
**Effect:** honest mid-run progress numbers in stage headers and PR trailers — a pure UX-trust fix.

## 6. PR-body strict-JSON generation failed once — seed **fabro-41b1** (open)
**What happened:** worker log 12:44:18: `PR content structured generation failed; retrying once without strict JSON output` — the retry saved PR #179, but a generation was wasted on a known-fragile strict parse.
**Change:** per fabro-41b1, make PR postlude body generation non-strict by default (`.fabro/` postlude pipeline).
**Effect:** one fewer wasted LLM call per run; removes a single point of PR-publish failure.

**What already worked (don't touch):** the tester fast path ("no crates touched", 3.7 s gate), closeout (0.36 s), journal compliance on every stage, first-pass approval driven by the inline PASS/FAIL report (reviewer used exactly 1 tool, $0.062), and full prompt caching (2.03 M cache-read tokens in the implementer). Notably, this run *built* the architect workflow (fabro-562a) whose analyze/file stages will convert in-run painpoints like #2b into tracker seeds automatically — the friction-to-seed pipeline this run's journal entries currently lack.
