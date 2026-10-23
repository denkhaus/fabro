# Revision — run 01M5KEMT84HHVYEZDS1KZC5S7E

- status reviewed: succeeded(completed)
- review: .fabro/reviews/develop/01M5KEMT84HHVYEZDS1KZC5S7E.md
- items filed: none — healthy verification-only pass (5 findings survived dedupe but the pass had 0 balance credit; all journaled below, 4 as overflow, 1 as overflow-dup)
- balance: 0 non-exempt items filed / 0 — no credit this pass (no same-pass stale/superseded closes; fabro-30a5's stranded claim is a closeout-park gap, not staleness, and the tracker binding outage blocks closes anyway)
- basis: run 01M5KEMT84HHVYEZDS1KZC5S7E, workflow version 81e060e628fa5a1f01ae93243062bb6e715eee4eff6cb277d877b78748a82136, commit a5669dce9974768c791df775a449b99e95d0ea0f
- revised_at_commit: a5669dce9974768c791df775a449b99e95d0ea0f (ADR-0015: engine drift signal for later judgement)

## Findings

Filing-time closeout re-check: degraded (tracker tools down — "no work project is bound to target repository denkhaus/fabro", 3rd+ consecutive pass); fallback grep of `.seeds/issues.jsonl` shows 0 items citing run `01M5KEMT84HHVYEZDS1KZC5S7E` — no closeout race. Overflow ledger `open` consulted: no open entry covers findings 2–5 below.

**1. Approved verification-only seeds can never close — the loop re-claims fabro-30a5 forever** — overflow-dup

- overflow-dup: Verification-only approved seeds park forever (open in 01M5K62554W7YCVZ7W4AT8YYWK.md)

Same theme already open in the ledger (closeout.nu `seed-demand-visible` parks verification-only approved seeds; PR #478 merged as b460ec604 yet fabro-30a5 stays open and the 6h stale-claim requeue re-arms it, ~$0.11 + ~2min + one PR per cycle). The new evidence strengthens the existing entry; no second open overflow filed.

**2. Park message prints a literal `($seed_id)` — interpolation escaping bug** — overflow

- overflow: Fix escaped interpolation in closeout park message — drop the backslashes on `.fabro/workflows/develop/scripts/closeout.nu:536` (`\($seed_id\)` renders literally; line 572 shows the correct form); effect: park notifications actually name the parked seed id instead of `($seed_id)`, prerequisite for fabro-9e2c being useful. Evidence: run events read "closeout: PARK — ($seed_id) NOT closed".

**3. Don't open a PR for a pass with no implementing diff** — overflow

- overflow: Gate run-PR creation on a non-empty seed-work diff or an actual close — engine postlude / `[run.pull_request]` config skips PR creation when the closeout parked or the diff is tracker-claim + journal only; effect: no more empty review-noise PRs from parked/no-op passes (PR #478 was created 04:02:12–04:02:56, model glm-4.7, for a 2-file/+9−1 diff that contained no implementing change; repeats every cycle until the verification-only park gap closes). Degraded dedupe: grep of `.seeds/issues.jsonl` shows no seed covering PR gating on parked passes.

**4. Reviewer prompt asserts a gate that never ran on this route** — overflow

- overflow: Scope the reviewer gate premise to implement runs — edit `.fabro/workflows/loop/prompts/reviewer.md:16` from unconditional "The loop gate was green" to "On implement runs the gate was green…; on verification-only runs no gate ran — see below"; effect: removes a false premise the reviewer used this run ("Gate: green by construction") to default-credit gating on a route where the tester node never fired, forcing reliance on `[by planner]` recorded checks. Degraded dedupe: no seed covers the reviewer's gate premise (fabro-9d26 closed = the fast path itself).

**5. Pin the canonical `fabro_run_search` query in the planner prompt** — overflow

- overflow: Pin the canonical fabro_run_search invocation in planner prompt — add the exact valid form (workflow `"loop"` + valid non-terminal statuses) to `.fabro/workflows/loop/prompts/planner.md` step 5, mirroring the seeds CLI "never invent flags" table; effect: removes ~2 wasted erroring round-trips per planner pass (planner's in-flight check sent `status ["queued","running","awaiting_input"]` → "unknown run status `queued`"; tool summary 3 calls, 2 errors) and keeps in-flight exclusion from silently degrading. fabro-1cde (error text names valid statuses) and fabro-6b58 (bound the check) are adjacent; neither pins the prompt-side query.

Dropped as duplicates (analyst-verified, fabro-9e2c re-verified open this pass in the degraded tracker): rec 2 (park terminal state) → duplicate_of: fabro-9e2c; rec 6 (planner firehose) → duplicate_of: fabro-c3b4; reviewer `cache_read: 0` observation → duplicate_of: fabro-944d; `fabro_run_set_title` length-example observation → duplicate_of: fabro-b529.
