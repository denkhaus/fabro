# Improve review — run 01M30CHJERR2APKJVY6D0YMBDV

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (0.05 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-20 21:54+0000 by revisor `fabro_ask`

---

All evidence is in. Here's the analysis, grounded strictly in what this run did.

## What actually happened in this run

From run events (seq 5–31): a conductor develop-leg (parent worker `01M30CF6AZK…`, per the `fabro_run_create` shape in `develop-leg.md`) spawned this run at 21:46:19. Timeline: 2.1s sandbox boot + depth-100 clone → `start` (0ms) → `tracker_guard` script **228ms, exit 0**, `preferred_label: "Tracker empty"` → exit at 21:46:25. **Zero LLM tokens, $0.00 stage cost** — the guard's fast-exit design worked exactly as intended (its cited basis: a planner lap once spent 27.7s/$0.0186 to learn the same fact). The only diff was the run's own 2-line journal file, committed (`29366f9e`) and **pushed twice** (seq 25, seq 28 — same sha) to a remote branch that got no PR. Total elapsed 5.7s; real work was 4% of it.

But from workspace file `.seeds/issues.jsonl`: the tracker is **not** drained — `seeds-facc` and `seeds-3791` are both **open** with label `ready-for-agent` and **no assignee**. The guard correctly fail-closed (only fabro-assigned seeds count), yet the run recorded plain `succeeded/completed` with zero signal that the park is assignment-gated. No agent node ran, so no prompting/gate/tool-use recommendation is groundable in this run — everything below is orchestration, graph-output, and infra.

Seed check: this repo's tracker holds exactly three seeds — `seeds-e218` (closed), `seeds-facc`, `seeds-3791`. All three target the seeds binary/format core; none covers orchestration, guard semantics, journaling, or push policy. (`fabro-*` ids cited in code comments resolve in the origin repo denkhaus/fabro, not here — unverifiable from this run.)

## Recommendations, by expected impact

**1. Make `tracker-guard` distinguish "drained" from "parked on unassigned backlog".**
- Change: `.fabro/workflows/develop/scripts/tracker-guard.nu` — when the fabro-assigned view is empty, also run one unfiltered `sd list --format json --limit 200` count and emit `open_unassigned: N` in the routing JSON (and into the stage journal).
- Grounding: guard routed "Tracker empty" (events seq 21–22) while two open ready seeds sat unassigned (`.seeds/issues.jsonl` lines 2–3, grep confirms no `assignee` field); the conductor leg is instructed to read this outcome as "the queue is done; the human seeds new demand" (`develop-leg.md` step 4) — false here, and the run's terminal record gives the user no way to tell.
- Effect: the run record and conductor journal say "2 open seeds unassigned — needs user assignment," converting an invisible park into an actionable signal; the dev line resumes once the user assigns `seeds-facc`.
- Seed: **new-seed justification** — no existing seed covers guard observability; `seeds-facc`'s acceptance only requires the tracker scripts keep *working* under the new binary, not new guard semantics.

**2. Conductor surveyor: probe the tracker before firing a develop leg.**
- Change: `.fabro/workflows/conductor/prompts/survey.md` step 1 — replace "default to Work — a cheap develop pass is fine even when the tracker turns out empty" with a pre-fire `sd list --format json --assignee fabro --limit 200` (+ `--status in_progress`) check in the surveyor's own sandbox; route "Work" only when non-empty.
- Grounding: this run is the direct product of that sentence — a full sandbox boot, clone, branch, commit, and two remote pushes (seq 25, 28) to answer a question 228ms of shell answered; surveyor's toolchain image already ships `sd`.
- Effect: no-op develop runs (≈6s infra + one orphan remote branch + 2 pushes each) drop to zero on drained/unassigned polls; burn-down latency unchanged when work exists.
- Seed: **new-seed justification** — all three tracker seeds target the binary/format, none the conductor's firing policy.

**3. Stop pushing journal-only run branches (and the duplicate second push).**
- Change: engine checkpoint policy (fabro repo) with the graph-side knob in `.fabro/workflows/develop/workflow.toml` `[run.run_branch]` — treat a diff whose only files match `.fabro/journal/**` as push-exempt, and skip the terminal push when the checkpoint already pushed the identical sha.
- Grounding: the entire pushed diff was the run's own 2-line journal (seq 23/29); commit `29366f9e` was pushed twice within 600ms (seq 25, 28); no PR followed (`pull_request: null`), so branch `fabro/run/01M30CHJERR2APKJVY6D0YMBDV` is a permanent orphan on origin — one per drained poll.
- Effect: drained exits leave zero remote branch churn; duplicate push eliminated even for real runs.
- Seed: **new-seed justification** — no existing seed covers checkpoint/push policy.

**4. Journal the guard's verdict data, not just node status.**
- Change: `.fabro/scripts/stage-journal.nu` — for command nodes, put the routing JSON (`preferred_label`, `degraded`, `requeued`, counts) into the stage record's `data` field.
- Grounding: this run's journal records for `start` and `tracker_guard` both carry `"data":{}` (workspace `.fabro/journal/01M30CHJERR2APKJVY6D0YMBDV.jsonl`); the guard computed degraded/requeued state into its 158-byte output blob, but nothing durable records it — a future *degraded* fail-open pass would leave the loop's own "silence is a missing report" rule violated at the only node that ran.
- Effect: drained/degraded/requeued passes leave an inspectable trail; the improve workflow (which scans run branches for journals) finally receives signal from no-op runs.
- Seed: **new-seed justification** — no existing seed covers the stage-journal hook's payload.

Not recommended: any change to the planner/implementer/reviewer prompts, gate, or tool lanes — no agent stage executed in this run, so there is no run evidence to ground such a change on; the prior run's journal (in this workspace) does contain implementer/tester painpoints (`verify.nu` hardcoded `lib/` layout, `dup-run-check.nu` fetch failure, `ml`/mulch unusable), but those belong to that run's record, not this one.
