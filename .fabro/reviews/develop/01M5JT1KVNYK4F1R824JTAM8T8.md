# Improve review — run 01M5JT1KVNYK4F1R824JTAM8T8

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: failed (22:00–22:03 Berlin on 2026-10-22, ~3 min, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 02:24+0200 by revisor `fabro_ask`

---

## What actually happened in this run (evidence base)

From run events + tracker + workspace journal: scheduled fire of the `Loop` workflow at 22:00:15; `start`→`env_guard`→`tracker_guard`→`preflight` all green in ~15s. The planner then worked for ~2.5 min (19 tool calls: `seeds ready` returned a **64-row / 9.2KB firehose**, examined `fabro-d48f`, **claimed it** — the tracker now shows fabro-d48f `in_progress`@loop, stranded by the dead run), then hit **429 code 1308 "limit reached, resets 2026-10-23 08:10:52" (~10h away)**. The engine ran **5 count-based attempts with ~40s backoff**, each spawning a **fresh session** that re-loaded the 28.7KB AGENTS.md and re-sent the full planner prompt, each dying on the same 429. The soft-exit edge (`planner -> exit [x.kind=soft]`) then routed correctly. Total: 220.6s wall, $0.144, no seed progress. Two extra glitches: all **3 `fabro_run_set_title` calls rejected** ("run title must be at most 100 characters"), and the terminal status **flipped `succeeded` → `failed(terminated)` 35ms apart** (seq 339→340, "Worker exited before emitting a terminal run event").

Tracker searches run for dedupe: `rate limit`, `fallback model`, `quota reset`, `title`, `worker exited`, `heartbeat`, `in-flight`, `requeue stale claim`, `d48f`, plus the full first-100 dump.

---

## Recommendations, by expected impact

**1. Honor the provider reset deadline instead of count-based retries (error handling, engine).**
What happened: 5 attempts × 40s backoff against a 10-hour quota window; attempts 2–5 each re-sent the full prompt + memory in new sessions (sessions `ses_8646…`, `ses_986d…`, `ses_be93…`, `ses_052a…` all visible in events).
Change: in the engine's LLM retry path, parse the reset timestamp from the 429 (code 1308) and immediately convert to park/soft-stop — and have the scheduler skip fires until reset.
Effect: this run would have soft-stopped at ~22:00:57 instead of 22:03:55; every scheduled fire during the ~10h window stops burning ~3 min + ~$0.10 + 3 duplicate sessions apiece.
Seed: **new seed justified** — the exact designs exist but are all *closed* (`fabro-a3d8` honor reset times/park-and-resume, `fabro-1f70` automation skips fires until reset, `fabro-183f` continue the session instead of re-sending the prompt); no open seed tracks that engine 0.362.0-fork.23 still exhibits the behavior, so file a regression/verify-deploy seed naming those three.

**2. Stop the worker-exit handler from overwriting a concluded terminal status (UX/engine).**
What happened: coordinator emitted `run.finished: success` (seq 338), platform recorded `succeeded` (seq 339), then `failed(terminated)` with the misleading "Engine error: Worker exited before emitting a terminal run event" (seq 340) — so the run reads as an engine crash when the real cause was a rate-limit soft-stop, and Slack (`run.completed` + `run.failed` both subscribed) double-notified.
Change: worker-supervisor exit path must respect an already-emitted terminal run event, and the terminal record must carry the root-cause failure class (rate_limit + reset time).
Effect: triage stops chasing a phantom engine crash; one truthful Slack message per terminal run.
Seed: display half is **fabro-0230** (open, "Surface root-cause error in terminal run status"); the overwrite race itself needs a **new seed** — searches for "worker exited" returned 0 matches and fabro-0230 covers only surfacing, not the race.

**3. Make the run-title instruction fit the 100-char cap (prompting).**
What happened: after claiming, the planner called `fabro_run_set_title` **3 times, 3 rejections** — "run title must be at most 100 characters" — because the prompt (and the tool's own description) ask for "a few sentences."
Change: `.fabro/workflows/loop/prompts/planner.md`, RUN TITLE section — prescribe "one clause, ≤90 characters, count before calling" (and fix the tool description's "few sentences" wording).
Effect: kills 3 wasted tool calls + model turns per claimed run; run tiles finally get titles (this run died titleless).
Seed: **fabro-b529** (open — same 100-char-cap rejection mechanism, seen from revisor select; this run is the loop-lane instance; **fabro-967e** is the open revisor sibling).

**4. Top-N ready view instead of the 64-row firehose (tool efficiency).**
What happened: the planner's first tracker call inlined **9.2KB / 64 seeds** into its context, though the preflight had already shortlisted 5 candidates.
Change: planner prompt command table in `.fabro/workflows/loop/prompts/planner.md` (and develop's twin) — consume the preflight candidate list first, fall back to a top-N `seeds ready` view only when needed.
Effect: ~9KB less context per planner pass, faster candidate triage, less drift down irrelevant rows.
Seed: **fabro-c3b4** (open, @loop — "Use a top-N sd ready view in the planner instead of the full firehose"); **fabro-c16d** (open) is the adjacent `--limit 50` footgun.

**5. Repair the in-flight guard's tool path (tool usage).**
What happened: step 5 (IN-FLIGHT EXCLUSION) made 3 `fabro_run_search` calls with **2 errors**; the only in-flight seed (fabro-3d9a) was actually caught by the preflight table, not the tool. (I could not read the error bodies — event search returned no content for those tool-error events — so the failure mode itself is unevidenced.)
Change: per **fabro-d62c** (open: loop lane run-level fabro tools run degraded) wire the binding properly, or move exclusion server-side per **fabro-997d**/**fabro-0f6e** (open).
Effect: duplicate-implementation guard stops failing ~2 of 3 invocations — the lane's core "no seed implemented twice" invariant currently rests on the preflight alone.

**6. Aggregate attempt metrics into the stage conclusion (observability).**
What happened: the planner's recorded stage metrics are **754ms wall / 0 tokens / tool_ms 0** — only the final failed attempt counts — while reality was ~191s, 29,725 input tokens, 366K cache-reads, $0.144 across 4 sessions and 19 tool calls.
Change: engine execution layer accumulates duration/usage across attempts (or emits per-attempt rows) into `step.finished`/the run conclusion.
Effect: per-stage cost/time tables become trustworthy — **fabro-1409** (open: append per-stage cost/time table to PR bodies) would currently render zeros for exactly the runs that waste the most.
Seed: **new seed justified** — no open seed covers attempt-metric aggregation (fabro-1409 is the consumer of the broken data, not the fix; searches found nothing else).

**7. Heartbeat on long stages and retry waits (UX).**
What happened: the planner was silent from 22:00:44 to 22:03:55 while cycling retry backoff; nothing visible until the terminal flip.
Change: notification hook (the run already has the `stage-journal`/`judgment-shadow` hook rail to hang it on) emitting a heartbeat on `retry.scheduled` and long stage waits, including the quota reset time.
Effect: humans see "attempt 3/5, quota resets 08:10:52" live instead of a dead-looking run and a post-mortem "terminated".
Seed: **fabro-b769** (open, @loop — "Add a long-stage heartbeat notification for run visibility").

---

One design note, no action needed: the stranded `fabro-d48f` claim will be recovered by tracker_guard's stale-claim requeue (threshold 6h, visible in its output; mechanism from closed **fabro-d9f7**) — that safety net worked as designed; if the 6h latency ever hurts, that's the seed to reopen.
