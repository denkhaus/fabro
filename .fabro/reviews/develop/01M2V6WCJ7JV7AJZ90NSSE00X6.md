# Improve review — run 01M2V6WCJ7JV7AJZ90NSSE00X6

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (21.9 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-18 22:14+0000 by revisor `fabro_ask`

---

## Run facts (grounding)

From the run's stage timings and conclusion (run events, `fabro_run_get`): wall 21m24s, active 1264s, cost $0.864. Breakdown: implementer 900s wall / $0.542 (366s inference vs **533s tool time**), tester gate 216.8s, planner 77.5s / $0.173, reviewer 64.8s / $0.149. All stages passed first-try; the friction is in build time, evidence format, and preflight signal quality — recorded as three journal painpoints (planner, implementer, reviewer) plus observed tool errors.

## Recommendations, by expected impact

**1. Warm the run container's Rust build — seed fabro-c643 (exists).**
What happened: the implementer journal records `cargo check -p fabro-server --tests` at **~205s cold vs ~24s warm**; the tester gate took 216.8s on a tree the implementer had just compiled; ~58% of run wall was compile/test. Change: the toolchain image / run environment per fabro-c643 (cargo-chef dependency layers or a persistent sccache volume). Expected effect: several minutes off every Rust seed run; this single change dwarfs all prompt-level tuning.

**2. Emit the evidence blob as plain-text multi-line — seed fabro-8cef (exists; fabro-9837 is the complement).**
What happened: reviewer journal — the 72.8KB evidence capture is "a single JSON-escaped line", grep/read_file paging returned wall-of-text, and the reviewer fell back to a python substring extraction (1 of its 7 shell calls errored; 64s inference for a checklist review). Change: `.fabro/workflows/develop/scripts/evidence.nu` writes sections (array / real newlines) instead of one escaped line. Expected effect: reviewers page sections directly; removes the escape-hatch class that seeds Verification-blocked risk on every large-diff review.

**3. Gate the preflight `in_flight` marker on run/PR state — new seed needed.**
What happened: planner journal — `planner-preflight.nu` marked the user-ordered priority-1 seed fabro-ab8e `in_flight` via run `01M2TS5BSB9ATK2GJGYA3Z2640` whose PR #242 was **rejected** and whose branch no longer exists. The planner burned ~60s and ~6 extra LLM/tool rounds (journal grep that 404'd, branch checks, git-log greps) adjudicating the stale marker before claiming (run events seq 44–83). Had it trusted the flag, the user-ordered seed would have been skipped. Change: in `.fabro/workflows/develop/scripts/planner-preflight.nu`, only mark `in_flight` when the run is non-terminal OR its PR state is open/unknown-and-branch-present. New-seed justification: no existing seed covers the preflight's terminal-state gating predicate — fabro-6b58 bounds the window, fabro-9372/fabro-4bb7 (name them as dependencies) fix the underlying projection gaps, but none change the marker's predicate itself.

**4. Fix the rust-style-guide hard-gate instruction to use the shell — seed fabro-d856 (exists).**
What happened: the implementer prompt says the FIRST action is reading `.fabro/skills/rust-style-guide/SKILL.md` "with `read_file`" — but that path is fs_hide-bound, and the implementer's tool stats show exactly **read_file: 1 call, 1 error** before it recovered via shell. Change: `.fabro/workflows/develop/prompts/implementer.md` (and reviewer.md per the seed) prescribes the shell read. Expected effect: removes a guaranteed failed tool call + recovery turn in every Rust pass.

**5. Name the upstream-ref procedure in PROJECT_FACTS — new seed needed.**
What happened: implementer journal + its 1 shell error — the brief's `git diff --numstat upstream/main..HEAD` fails with "bad revision" because the sandbox has no `upstream/main` ref; the workaround was `git fetch origin main` then diff against FETCH_HEAD (170291b9f). Change: add one bullet to the PROJECT_FACTS block (`.fabro/workflows/develop/prompts/` facts include) stating the fetch-then-FETCH_HEAD form for upstream comparisons. Expected effect: every fork-strategy seed stops rediscovering this. New-seed justification: closest existing seed (fabro-1128) is revisor-scoped and about timeouts, not the missing ref/workaround in develop.

**6. Top-N `sd ready` view — seed fabro-c3b4 (exists).**
What happened: the planner's first shell call returned **200 seeds / 29.3KB of stdout** it had to page through (run events seq 38–39); planner conversation tokens were 37.7k of its 49k input. Change: planner uses a bounded top-N listing (seed's scope). Expected effect: smaller planner context, faster claim decisions, less misreading risk on candidate ordering.

**7. Resolve crate-relative seed anchors — seed fabro-7611 (exists; planner observation this run cites it).**
What happened: the preflight flagged **4 `missing_file` anchors** on the claimed seed (`server/tests.rs`, `server/handler/wait.rs`, `tests/it/api/run_wait.rs` — all real under `lib/apps/fabro-server/`), forcing the planner to verify each by hand (seq 63–64). Change: `anchor_check.nu` resolves crate-relative paths per the seed. Expected effect: `anchors_ok` becomes trustworthy; planner skips the manual verification shell calls.

## Not recommending

Gate design, cycle guards, claim_check, closeout, and the single-pass loop all worked as designed this run (0 retries, 0 deadlocks, ~90ms claim check) — the graph itself needs no structural change; the levers are the environment (1), the evidence pipe (2), and preflight signal quality (3, 7).
