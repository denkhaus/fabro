# Improve review — run 01M5KEMT84HHVYEZDS1KZC5S7E

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded(completed) (111.5s, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 04:35+0000 by revisor `fabro_ask`

---

All evidence is in. Here's what actually happened in run `01M5KEMT84HHVYEZDS1KZC5S7E`, and what to change — ordered by expected impact.

## Run recap (from run events + conclusion)
A verification-only pass: planner claimed **fabro-30a5** (a no-op deferral record — its action is literally "none"), routed `Verification-only` → evidence → reviewer **Approved** → closeout **PARKED the seed** ("no implementing change for the seed's demand") yet the run concluded **succeeded(completed)** and opened **PR #478** containing only the tracker claim line + journal. Wall 111.5s, cost $0.1096 (planner $0.0844 / 58.4s = 77% of both). No gate ever ran (tester node never visited). Journal recorded **zero painpoints** despite the park.

## Recommendations

**1. Approved verification-only seeds can never close — the loop will re-claim fabro-30a5 forever.** (graph design + script)
- Evidence: reviewer verdict `approved` (run events); closeout output "PARK — … NOT closed … no implementing change"; `seed-demand-visible` in `.fabro/workflows/develop/scripts/closeout.nu:167-174` token-matches the seed **title** against the claim-base diff — on the verification-only route the diff is empty **by design**, so it parks unconditionally. The seed sat at the top of `seeds ready` this run; the 6h stale-claim requeue re-arms it. Cost: ~$0.11 + ~2min + one PR per cycle, indefinitely.
- Change: one unit (ADR-0008): planner emits a `verification_only` context flag (add to `.fabro/workflows/loop/schemas/planner-output.schema.json`), and `seed-demand-visible`/the park gate in `closeout.nu` treats that flag + `review_verdict=approved` as demand-visible.
- Effect: fabro-30a5 closes next pass; the repeat-loop burn stops.
- Tracker: **new item** — searches `closeout verification-only park` (0), `closeout park` (only fabro-9e2c, notification visibility), `verification-only` (7 hits, none touch closeout) show no existing item covers the park-gate exemption.

**2. A park reports as green "Seed closed" — make the terminal state tell the truth.**
- Evidence: `closeout.nu:539` parks with `exit 0`; the graph's only success edge is `closeout -> exit [label="Seed closed"]`, so this run concluded `succeeded/completed` and notified `run.completed` on #dev-fabro while the seed stayed open.
- Change: in `.fabro/workflows/loop/workflow.fabro`, have the park path emit a distinct `preferred_next_label` (e.g. "Parked") with its own soft-exit edge (mirrors develop), instead of riding the green edge.
- Effect: kanban/Slack show "parked, seed open" instead of green; humans can break loops like #1.
- Tracker: **fabro-9e2c** ("Surface the closeout park state in the terminal event and notification", open) — this is its mechanism.

**3. The park message prints a literal `($seed_id)` — interpolation escaping bug.**
- Evidence: closeout output in run events reads "closeout: PARK — ($seed_id) NOT closed"; `closeout.nu:536` uses `\($seed_id\)` (escaped parens render literally), unlike line 572's correct `($seed_id)`.
- Change: drop the backslashes on line 536.
- Effect: park notifications actually name fabro-30a5; prerequisite for fabro-9e2c being useful.
- Tracker: **new item** — one-line script bug observed in this run's output; no search hit covers it (`closeout park` → only fabro-9e2c).

**4. Don't open a PR for a pass with no implementing diff.** (UX)
- Evidence: PR #478 was created (04:02:12–04:02:56, model glm-4.7) for a 2-file/+9−1 diff that is tracker claim + journal only — pure review noise, and it repeats every cycle until #1 is fixed.
- Change: gate PR creation (engine postlude / `[run.pull_request]`) on a non-empty seed-work diff or an actual close at closeout.
- Effect: no more empty PRs from parked/no-op passes.
- Tracker: **new item** — searches `pull request` (5 hits, all other topics) and `empty PR parked seed` (0) found nothing covering PR gating.

**5. Reviewer prompt asserts a gate that never ran on this route.** (prompting soundness)
- Evidence: `.fabro/workflows/loop/prompts/reviewer.md:16` states unconditionally "The loop gate was green (the Evidence step only runs after a green gate)" — but line 17 says tester never runs on verification-only, and this run's reviewer explicitly reasoned "Gate: green by construction (Evidence only runs after a green gate)" while no tester visit exists in the event stream.
- Change: scope line 16 to implement runs ("On implement runs the gate was green…; on verification-only runs no gate ran — see below").
- Effect: removes a false premise the reviewer can use to default-credit gating; forces reliance on `[by planner]` recorded checks.
- Tracker: **new item** — searches `verification-only`, `reviewer verification-only` (only closed fabro-9d26, the fast-path itself) cover nothing about the reviewer's gate premise.

**6. Planner paid a 65-row firehose it didn't need.** (cost — planner was 77% of run cost)
- Evidence: `seeds ready --assignee loop --limit 200` returned 65 issues / 9,491 bytes into context (run events, planner shell call) although the preflight had already narrowed candidates to 5 (`output.preflight` inline).
- Change: in `.fabro/workflows/loop/prompts/planner.md` step 1, treat the preflight candidate table as the primary candidate source and demote `seeds ready` to fallback.
- Effect: cuts planner input tokens and latency every pass (this stage: 26.9k input tokens, 58.4s, $0.084).
- Tracker: **fabro-c3b4** ("Use a top-N sd ready view in the planner instead of the full firehose", open, @loop).

**7. Pin the canonical `fabro_run_search` query in the planner prompt.** (tool efficiency / error handling)
- Evidence: planner's in-flight check called `fabro_run_search` with `status ["queued","running","awaiting_input"]` → error "unknown run status `queued`"; tool summary: 3 calls, 2 errors. `planner.md:33` names the tool but gives no argument form (unlike the seeds CLI "never invent flags" table).
- Change: add the exact valid invocation (workflow `"loop"` + valid non-terminal statuses) to `planner.md` step 5.
- Effect: removes ~2 wasted erroring round-trips per planner pass and keeps the in-flight exclusion from silently degrading.
- Tracker: **new item** — fabro-1cde (error text should name valid statuses) and fabro-6b58 (bound the check) are adjacent but neither pins the prompt-side canonical query.

**Two smaller observations (both would need new items; low priority):** the reviewer stage ran with `cache_read: 0` (15,702 input tokens at full price vs the planner's 156,672 cached — engine-side preamble caching for the reviewer session is worth an item), and the planner's first `fabro_run_set_title` attempt was rejected (2 calls, 1 error — almost certainly the >100-char limit the prompt already warns about; a length example in `planner.md` would prevent the retry).

**What I could not inspect:** PR #478's rendered body on GitHub (only the creation event), and whether the engine exposes a reviewer-preamble cache knob (no such attribute appears in the graph source).
