# Improve review — run 01M2YMRJQZMC6SVTQJ3JC49CWC

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (5.0 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-20 05:41+0000 by revisor `fabro_ask`

---

All evidence is in. Here are the recommendations, ordered by expected impact, each grounded in this run's events (run 01M2YMRJQZMC6SVTQJ3JC49CWC, seed fabro-a81d, total 285s wall / $0.373 / zero retries — a clean first-pass run, so everything below is recurring overhead, not failure recovery).

---

**Context: where the run's time and money went** (from run events / conclusion): planner 114.8s / $0.255 (**68% of run LLM cost**), implementer 99s / $0.089 (of which tool time was only 1.1s), reviewer 15.4s / $0.029 with **zero tool reads** (evidence arrived inline at 12.6KB — the fabro-1e9f budget raise worked as designed), scripts 35s total. No gate bounce, no retries.

## 1. Add an "upstream-gated" arm to the planner preflight — the #1 recurring planner tax
- **What happened**: The planner burned seq 53–82 (~40s, ~$0.12) re-deriving that fabro-af22 (High, tops `sd ready` since 2026-08-22) is gated on upstream PR fabro-sh/fabro#784 — `sd show`, git log, three rg/grep probes, then skip. The preflight verdict table said only `clean` because it checks landed-ness, not claimability-by-external-gate. af22 will top the list **every run** until #784 merges, so this ~25–40% of run wall/cost recurs daily.
- **Change**: `.fabro/workflows/develop/scripts/planner-preflight.nu` — add a per-candidate `upstream_gated` flag from a mechanical body-marker grep (`UPSTREAM PR OPEN`, `close when #<n> is merged`); planner skips flagged candidates like `in_flight`, exactly per the standing policy already in `planner.md` ("mechanically-checkable invariants land in planner-preflight.nu, NEVER as prose").
- **Expected effect**: removes ~40s / ~$0.12 per develop run until af22 closes; preflight already fetches the bodies, so the arm costs milliseconds.
- **Seed**: new-seed justification — no open/closed seed covers claimability pre-screening for upstream-gated bodies (fabro-a32f = landed-history check, fabro-0da8 = tracker emptiness, fabro-6b58 = time-window bounding; verified by grep of `.seeds/issues.jsonl`).

## 2. Block/refile seeds whose change target lives outside the repo (fabro-23d4 is the live case)
- **What happened**: The planner spent seq 83–100 (~31s, ~$0.055) discovering that fabro-23d4's target (`sd note`) is the external npm seeds CLI (`which sd` → `/mise/bun-global/bin/sd`, `ls bin/`, `sd --help`), then journaled "needs an upstream-scoped refile or a wrapper decision" — a journal observation that dies with the run, so the next planner re-derives it.
- **Change**: apply the fabro-a285 blocked-external pattern to fabro-23d4 now (tracker action), and add an intake arm so the class stops recurring: a seed whose named change target (binary/path) has no source in this repo gets flagged by the preflight (same table as #1) or by the revisor intake lint.
- **Expected effect**: removes ~31s / ~$0.055 per run until 23d4 is refiled; prevents the class for future external-tool seeds.
- **Seed**: new-seed justification — fabro-3839 (open) lints basis refs/tool-name typos only; fabro-a285 is a one-off per-seed refile, not the intake rule; nothing covers "change target must resolve in-repo" (verified by grep).

## 3. Implement open seed fabro-6997 — the rg `-rn` footgun fired in the *planner* this run
- **What happened**: Planner seq 65 ran `rg -rn "find_skill_references|expand_skill" /workspace/fabro/lib` — `-r` is replace-mode (the exact documented footgun) — then burned a recovery turn at seq 70 ("rg with -rn is wrong"). The one-liner exists in `implementer.md` only; `planner.md` doesn't carry it.
- **Change**: `.fabro/workflows/develop/prompts/planner.md` — add the two fabro-6997 footgun one-liners (`rg -r` replaces, never `rg -rn`; fs_hide glob returns empty).
- **Expected effect**: kills a wasted LLM turn per occurrence and the replace-mode correctness risk (harmless here only because there were no matches).
- **Seed**: **fabro-6997** (open).

## 4. Brief-scope rule: when a criterion demands cross-file consistency, name the exhaustive mirror list — no narrower parenthetical
- **What happened**: The brief's criterion 4 said "any other prompt or doc passage… **(grep the develop prompts)**" — contradictory scopes. The implementer spent a single 43s reasoning burst (05:34:16→05:34:59, 2,483 reasoning tokens, ~25% of its inference) deliberating whether the revisor mirror at `.fabro/workflows/revisor/prompts/file.md:93` was in scope, and the reviewer then re-adjudicated the same scope-creep question (its journal observation says the "explicit grep clause" is the only thing that made it unambiguous). Both costs trace to one ambiguous bullet.
- **Change**: `.fabro/workflows/develop/prompts/planner.md` step 6/7 — a consistency criterion must enumerate its full search surface (here: "develop prompts + the revisor `file.md` mirror") or the parenthetical must match the criterion scope; never both. Fold this run's implementer journal observation as the basis.
- **Expected effect**: removes the ambiguity-deliberation burst in the implementer and the scope adjudication in the reviewer per prompt-mirror seed (~$0.03–0.05 combined).
- **Seed**: new-seed justification — no seed covers brief-internal scope contradictions (fabro-dad8 = probe contradictions; fabro-7773 = seed-body lint at intake; verified by grep).

## 5. Implement open seed fabro-c3b4 — the `sd ready` firehose dominated planner context
- **What happened**: `sd ready --assignee fabro --limit 200` returned **200 issues / 28KB** into the planner conversation (seq 46, `stdout_truncated: true`); the session then climbed to ~64k tokens, and every subsequent turn re-carried it (591k cumulative cache-read tokens across 12 LLM calls). The planner only ever needed the top handful it adjudicated.
- **Change**: per fabro-c3b4, feed the planner a top-N view (preflight already has the list — it can pass the top candidates inline, shrinking the first `sd ready` to a bounded probe).
- **Expected effect**: cuts planner per-turn context ~40%+ on candidate-selection laps; compounds with #1 and #2.
- **Seed**: **fabro-c3b4** (open).

## 6. Implement open seed fabro-6b58 — the in-flight check pulled all 109 develop runs unbounded
- **What happened**: The planner called `fabro_runs_list` with only `workflow: "develop"` (seq 51) — no `created_since`, no self-exclusion — and got 109 full run records it then had to reason over, including the null-PR-state interplay it journaled.
- **Change**: per fabro-6b58, add `created_since` (stale-claim window, e.g. 6h matching the tracker guard) plus explicit self-run exclusion to the step-4 call in `planner.md`.
- **Expected effect**: shrinks a large tool payload and the reasoning over it; removes the self-run noise it had to mentally filter.
- **Seed**: **fabro-6b58** (open).

## 7. Allowlist by-design gate warnings — 11 of 11 warnings in this run's gate output are noise
- **What happened**: The tester output (from run events) emitted 10 "routing-named property … activates routing semantics" warnings for the two *intentional* routing schemas (`planner-output.schema.json`, conductor `develop-output.schema.json`) plus the `project-facts.md` date-pin ">45 days — still load-bearing?" warning. `prompt-lint: ok — 43 files, 11 warnings`. A real new warning would be invisible in this noise, on every gate run.
- **Change**: `.fabro/scripts/prompt-lint.nu` — an intentional-routing marker (e.g. `"x-routing-intent": true`) that silences the routing-named warning for those two schemas, and a load-bearing annotation for the nightly pin in `project-facts.md`.
- **Expected effect**: gate warnings become signal; a genuine schema regression actually gets read.
- **Seed**: new-seed justification — fabro-a211 (closed) *introduced* these warnings as authoring guidance but left intentional schemas warning forever; fabro-8275 downgrades a different by-design warning; none covers this allowlist (verified by grep).

## 8. Implement open seed fabro-4bb7 — backfill PR state so planners stop inferring mergedness
- **What happened**: This run's own PR #314 has `state: null` (from run events), and the planner's journal observation 3 records exactly the downstream cost: "PR states show null for several succeeded runs but their commits are in base (e.g. PR 312) — consistent with the fabro-4bb7 backfill gap." Every planner re-performs this inference, and a wrong guess is a false in-flight skip or a wrongful claim.
- **Change**: per fabro-4bb7, backfill `pull_request.state` for terminal-but-unmerged runs in the `fabro_runs_list` projection.
- **Expected effect**: in-flight exclusion becomes a table lookup instead of an LLM judgment; removes a recurring mis-skip risk.
- **Seed**: **fabro-4bb7** (open).

## 9. Implement open seed fabro-652d — batch checkpoint pushes
- **What happened**: The engine committed+pushed the run branch after **every** stage boundary (git.push events at seq 25, 33, 122, 130, … — 10 pushes total), contributing most of the ~22s gap between 285s wall and 263s active time.
- **Change**: per fabro-652d, push at terminal/soft-exit boundaries only.
- **Expected effect**: ~15–20s per run; small, but free.
- **Seed**: **fabro-652d** (open).

---

**What NOT to change** (validated this run, from run events): the evidence pipe (12.6KB capture rendered inline under the 16KB cap; reviewer needed zero tools at $0.029 — cheapest stage), the gate's touched-crate derivation ("no crates touched" → lint+fmt only, 23s), the claim-contract chain (claim_check 84ms), and the markdown-only cost-tiering in the implementer's verify lane (`just verify implementer` exited 0 without a build probe). No recommendation there — those seeds (fabro-1e9f, fabro-574d, fabro-6e7f, fabro-c42f) are performing as designed.

**Could not inspect**: the persisted worker log (`fabro_run_logs` not consulted for this analysis — all findings above come from run events, checkpoints, and workspace files, which were sufficient); and the planner's remaining events beyond seq 171 were not needed since the implementer's full tool sequence was captured through the edit application.
