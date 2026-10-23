# Improve review — run 01M5KNGHH0ZAPKSD07RX7N8CKS

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (deadlock-routed Blocked, ~112s wall, $0.093 planner, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 09:08+0200 by revisor `fabro_ask`

---

## What actually happened in this run (evidence base)

From run events and the run projection: the scheduled loop run passed its deterministic prefix in ~4s (env_guard 42ms, tracker_guard 383ms → "Tracker non-empty", preflight 3.5s → 5 clean candidates), then the **planner burned 105.6s wall / 104.5s inference / $0.093 (25.4k input + 191k cache-read tokens)** — 77% of the whole run — only to route **Blocked**: every `fabro_work_next`/`fabro_work_list` call failed with *"no work project is bound to target repository denkhaus/fabro"*, an outage already documented in `.fabro/revisions/01M5K62554W7YCVZ7W4AT8YYWK.md` (planner found that doc via `rg` 10s after the first error, then kept investigating for ~66 more seconds: a second `fabro_work_next`, 6 shell calls, 1 `fabro_run_search`). The Blocked edge is `x.kind="deadlock"`, yet the run concluded **status "succeeded"** and still **opened PR #487** whose entire diff is 5 appended journal lines (from checkpoint diff summaries + `.fabro/journal/01M5KNGHH0ZAPKSD07RX7N8CKS.jsonl`, confirmed by reading the file). Root cause of the wasted pass: `tracker-guard.nu` reads the **legacy `seeds` file tracker** and is explicitly fail-open (header: "the planner adjudicates the residue") — so it reported "Tracker non-empty" from `.seeds/issues.jsonl` while the central tracker the planner must claim through was dead.

Tracker searches run for dedupe: `targetRepo`, `outage`, `bind`, `unbound`, `work project`, `tracker`, `guard`, `blocked`, `degraded`, `journal`, `deadlock`, `fast-exit`, `pull request journal`, `no work`.

---

## Recommendations, by expected impact

**1. Add a central-tracker health probe + "Tracker unreachable" exit arm to the deterministic prefix.**
- Change: in `.fabro/workflows/develop/scripts/tracker-guard.nu` (shared by the loop's `tracker_guard` node), add an arm that probes the central tracker binding (the same surface `fabro_work_next` needs); on the binding error, emit a new `preferred_next_label "Tracker unreachable"` and wire `tracker_guard -> exit [x.kind="soft"]` for it in `.fabro/workflows/loop/workflow.fabro` (and develop's graph, since the script is shared — one file, fix lands once).
- Expected effect: the scheduled loop stops spending a full planner pass ($0.093 + ~2 min per fire, recurring on every automation tick until the server-side binding is fixed — the outage already spans multiple runs per the revisions doc) to rediscover a mechanically detectable outage; blocked passes become a ~4s, zero-token exit with a journaled painpoint.
- Tracker: **new item** — none of `targetRepo`/`outage`/`bind`/`unbound`/`work project`/`guard` searches found an open seed for detecting the central-tracker outage deterministically; closest are the migration epic `fabro-a33e` (build-out, not this guard arm) and `fabro-e1ee` (preflight-failure soft-exit fork — adjacent graph-design question, not tracker-health detection).

**2. Teach the planner prompt to fail fast on the known binding error.**
- Change: in `.fabro/workflows/loop/prompts/planner.md` (the GRANT step), add one rule: if `fabro_work_next` returns the "no work project is bound…" error, do **not** retry the call or search for the project id — route Blocked immediately, citing the revisions doc. This run's trace shows the model reasoning "Maybe search runs to find project id? … check journal or scripts for project id" and running a redundant second `fabro_work_next` plus 6 shell calls after the outage was already identified. Note the provider also dropped the tool-error flag (`unsupported_control` warning, pebble seq 129), so the rule must match on the error *text*, which is stable.
- Expected effect: even before rec 1 lands (or for outage classes the guard can't probe), a blocked planner pass collapses from ~105s/$0.093 to roughly the 29s initial-reasoning minimum — the post-error 77s investigation loop disappears.
- Tracker: **new item** — no open seed covers prompt-level fail-fast on tracker-tool death (`outage`, `blocked`, `degraded` searches: `fabro-2afd` is closed and about absent preflight; `fabro-5b0a` is about surfacing degraded checks in the summary; `fabro-d62c` is about enabling run-level tools — none covers this).

**3. Suppress PR creation (and the extra PR-writing model call) on blocked/deadlock exits with journal-only diffs.**
- Change: this run pushed branch `fabro/run/…` and opened **PR #487 via a `zai:glm-4.7` call** (from run projection, `pull_request_creation`) for a diff that is exclusively `.fabro/journal/<run_id>.jsonl` (+5 lines). Extend the journal-only suppression to the deadlock/soft-exit conclusion path (engine-side publish step; the graph-side counterpart is the exit-kind handling in `.fabro/workflows/loop/workflow.fabro`).
- Expected effect: no PR noise and no second LLM call per no-op pass; on a scheduled loop during a multi-run outage this multiplies (per `fabro-5a3f`, journal-only PRs were already ~23% of run-PR volume in the revisor lane).
- Tracker: **`fabro-5a3f`** (open, needs-user — "No-op … runs publish journal-only PRs … accept vs suppress fork"); extend its scope from revisor to the loop lane. `fabro-9f97` (closed, "Skip PR creation when the run diff is journal-only") evidently didn't cover the deadlock-exit path — this run is the regression evidence.

**4. Make deadlock-exited runs read as blocked, not "succeeded".**
- Change: the planner routed `Blocked` on the `x.kind="deadlock"` edge ("needs a human"), yet the run's terminal status is `succeeded/completed` and Slack `#dev-fabro` got a `run.completed` event (from run spec) — a human scanning the channel sees green. Classify deadlock exits as a distinct terminal status at the engine's conclusion fold (and render it distinctly in the web header).
- Expected effect: the needs-user signal the graph comment promises ("deadlock keeps the seed open for a human") actually reaches the human; outage-class deadlocks stop hiding inside green notifications.
- Tracker: **`fabro-8cd3`** (open — render soft_stop/deadlock distinctly; this run adds the inverse case: deadlock rendered as *succeeded*). Engine-side classification was `fabro-08b` (closed, "boundary state needed") — this run shows the deadlock exit still concluding green, so the closed item's fix didn't cover this edge kind.

One non-issue worth keeping as-is: token efficiency otherwise is good — 191k of 216k billed tokens were cache reads, and the deterministic prefix is fast; the cost problem is confined to the wasted blocked passes addressed above.
