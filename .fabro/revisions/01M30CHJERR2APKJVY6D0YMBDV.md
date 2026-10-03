# Revision — run 01M30CHJERR2APKJVY6D0YMBDV

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M30CHJERR2APKJVY6D0YMBDV.md
- seeds filed: none — zero filing credit this pass (ADR-0022: no same-pass stale/superseded closes)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M30CHJERR2APKJVY6D0YMBDV, workflow version e3c92fec9a9a6fd9256277d60e06e12dddcacf6779280c9df5c2b2825b813ea8, commit b70853402f4d7ef86f442a0e7419f570fb11188a
- revised_at_commit: b70853402f4d7ef86f442a0e7419f570fb11188a (ADR-0015: engine drift signal for later judgement)

## Findings

### 1. Conductor survey: probe assigned seeds before firing a develop leg

- overflow: Conductor survey pre-fire probe — in `.fabro/workflows/conductor/prompts/survey.md` (item 3, line 16) replace the "default to Work" guidance with a pre-fire check in the surveyor's own sandbox (an sd list filtered to the fabro assignee plus in-progress status); route Work only when non-empty, otherwise journal the cause and route nothing; effect: no-op develop runs drop to zero on drained or unassigned polls (basis: run 01M30CHJERR2APKJVY6D0YMBDV was a full no-op leg — sandbox boot, clone, branch, two remote pushes of sha 29366f9e, no PR — to learn in 228ms what tracker_guard exits with; dedupe: fabro-12fe is the merge-leg pre-fire check, fabro-deb5 survey visibility, fabro-52a9 wording only).

### 2. Engine: stop pushing journal-only run branches and the duplicate identical-sha push

- overflow-dup: Skip PR creation for bookkeeping-only run diffs (open in 01M2ZHYDC6T4NDJX8B3VKBJQ9K.md) — the open overflow covers the PR-skip half; this finding adds push-exemption for `.fabro/journal/`-only diffs in `lib/components/fabro-workflow/src/pipeline/publish.rs` plus a graph knob in `.fabro/workflows/develop/workflow.toml`, and skipping the terminal push when the checkpoint already pushed the identical sha (basis: this run's commit 29366f9e was pushed twice within 600ms to a PR-less branch; dedupe: fabro-9f97 closed covered PR-skip only, fabro-652d is push cadence). Consume as an extension arm of that overflow when balance allows.

### 3. Stage-journal hook: record command-node routing JSON in the stage record data field

- overflow: Stage-journal routing payload — in `.fabro/scripts/stage-journal.nu`, for command nodes put the routing JSON (preferred_label, degraded, requeued, counts) into the stage record's data field instead of `data:{}`; effect: drained, degraded, and requeued exits leave an inspectable trail (basis: run 01M30CHJERR2APKJVY6D0YMBDV's start and tracker_guard journal records carry data:{} while the guard computed a 158-byte verdict blob nothing durable records; dedupe: fabro-d308 covers the hook's fault-tolerance, not its payload).
