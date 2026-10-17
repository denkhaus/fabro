# Improve review — run 01M536GGFVG16CGN8WPK05WYS7

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (2.4 min, revisor pass — reason and cost in run detail)
- generated: 2026-10-17 19:06+0000 by revisor `fabro_ask`

---

# Recommendations for the Loop workflow — grounded by run 01M536GGFVG16CGN8WPK05WYS7

**Dedupe note (important):** all five live tracker searches I ran (`closeout verification-only park`, `run_search in-flight`, `top-N ready firehose`, `sweep merged run diff superseded`, `commit message seed id reference`) failed with **401 auth errors** — the live tracker was not inspectable this turn. Dedupe below is against the authoritative in-run snapshot: the planner's `seeds ready --assignee loop --limit 200` output (64 seeds with ids/titles) captured in this run's event stream. Treat new-seed calls as needing one more live-tracker check before filing.

**What actually happened (from run events + stage outputs):** scheduled loop run, path `start → env_guard → tracker_guard → preflight → planner → evidence → reviewer → closeout → exit`, wall 163 s, cost $0.159 (planner 86.6 s / $0.116; reviewer 43.5 s / $0.043). The planner routed **Verification-only** on fabro-41fa (fix `c5045e5ef` already in base via PR #418, commit message carrying no seed-id), the reviewer **Approved**, and then closeout **PARKED** the seed: "no implementing change for the seed's demand" — which is true *by definition* on this route. The run still concluded green and opened PR #420 with a no-op diff (2 files, +9/−1: tracker claim + journal).

---

### 1. Closeout's park gate contradicts the Verification-only route (highest impact)
- **Evidence:** closeout stage output parks fabro-41fa despite `review_verdict=approved`; the park gate in `closeout.nu` (~lines 493–505, fabro-9967/fabro-02c4 class) requires an implementing diff vs claim-base — a verification-only run never has one. Planner journal explicitly predicted this: "closeout should close on reviewer approval citing c5045e5ef."
- **Change:** in `.fabro/workflows/develop/scripts/closeout.nu`, let the park gate accept an approved verification-only proof — pass the route/verdict into closeout (the loop graph's `closeout` node in `.fabro/workflows/loop/workflow.fabro` already has `stdin_source="current_seed_id"`; add the route key or read `review_verdict` from context) and skip the diff-demand check when the planner's recorded-checks section exists and the reviewer approved.
- **Expected effect:** approved verification-only seeds actually close; kills a repeat ~2.5-min/$0.16 verification cycle per parked seed per scheduled run, and stops no-op PRs like #420.
- **Seed:** *new-seed justification* — no seed in the 64-entry snapshot touches closeout park semantics; closest is **fabro-ab38** (sweep open seeds against merged run diff), which addresses the sibling problem (landed-without-seed-id fixes stranding seeds) but not the park-gate route conflict. File both angles separately; ab38 already exists.

### 2. `fabro_run_search` is pinned on the planner node but absent from the session
- **Evidence:** graph pins `x.fabro_tools="fabro_run_search"` on `planner`, but the planner journal records "fabro_run_search tool absent in this planner session (in-flight exclusion arm degraded)" and the `attractor.tools` event lists zero fabro tools. The run spec (`run.created`, automation `loop@fabro`) shows run-level `agent.fabro_tools: false` — a likely override of the node pin.
- **Change:** flip `agent.fabro_tools` for the loop automation spec (or fix node-pin precedence in the engine); verify in `.fabro/workflows/loop/workflow.fabro` planner node.
- **Expected effect:** the in-flight exclusion arm actually checks open PRs/non-terminal runs — this run, fabro-8932/fabro-a111 were excluded only via preflight's `in_flight` flag, leaving the PR-visibility half of the guard dead on every run.
- **Seed:** *new-seed justification* — **fabro-6b58** (bound the in-flight check: created_since + self-exclusion) polices the check's scope, not the missing tool; nothing in the snapshot covers tool absence.

### 3. Planner context firehose: full `seeds ready` list + static preflight legend every pass
- **Evidence:** planner input 29,488 tokens / 239k cache-read / 85 s inference for a low-effort routing decision; the `seeds ready` call inlined 64 seeds (~9.3 KB stdout, event `ToolProcessCompleted`), and preflight's routing JSON is 3.0 KB of which ~1.9 KB is a **static legend** re-inlined every run.
- **Change:** apply **fabro-c3b4** ("Use a top-N sd ready view in the planner instead of the full firehose") in `.fabro/workflows/loop/prompts/planner.md`; secondarily move the legend out of `planner-preflight.nu`'s routing output into the prompt (it's documentation, not per-run data).
- **Expected effect:** the firehose is a large fraction of the planner's 18.6k conversation tokens — trimming it cuts per-pass latency/cost on every scheduled run.
- **Seed:** **fabro-c3b4** (known, open in the loop queue).

### 4. Park message never names the seed — interpolation bug
- **Evidence:** closeout output prints the literal `($seed_id)` ("closeout: PARK — ($seed_id) NOT closed…"). From workspace file `closeout.nu` line 498: `$"closeout: PARK — \( $seed_id \) NOT closed…"` — the escaped parens suppress interpolation in the nu `$"…"` string.
- **Change:** unescape to `($seed_id)` in `.fabro/workflows/develop/scripts/closeout.nu:498` (keep the intentional literal parens on the other clauses).
- **Expected effect:** park notes in the journal/run output identify the parked seed, which is the whole point of the park mechanism for the next cycle — one-line fix, compounding with rec 1.
- **Seed:** *new-seed justification* — no seed in the snapshot covers closeout message interpolation.

### 5. Parked runs read green: exit label, notification, and PR don't distinguish "parked"
- **Evidence:** closeout exited via the unconditional `closeout -> exit [label="Seed closed"]` edge; run status `succeeded`, `run.completed` fired to #dev-fabro, and PR #420 (auto-merge squash) shipped a no-op diff — while the seed stayed open.
- **Change:** in `.fabro/workflows/loop/workflow.fabro`, have closeout emit a `preferred_label="Parked"` and route it to a distinct exit kind (e.g. `x.kind="soft"`, mirroring the "Closeout failed" edge).
- **Expected effect:** terminal status/Slack/PR reflect the parked state instead of green-washing it — a human scanning notifications can see the loop is spinning on a stuck seed.
- **Seed:** *new-seed justification* — nothing in the snapshot covers exit-kind/UX for parked closes (snapshot's UX-adjacent seed fabro-b769 is long-stage heartbeats, different problem).

### 6. Prompting: port the fresh-cycle "skip dead context reads" line to the loop planner
- **Evidence:** the planner's first tool call was an exploratory `ls .fabro/workflows/loop/ … | head -20` layout probe (event stream, planner transcript) — dead context PROJECT_FACTS largely already implies.
- **Change:** apply **fabro-a08a** ("visit-1 fresh-cycle line to skip dead context reads", currently develop-only, assigned @loop) to `.fabro/workflows/loop/prompts/planner.md`.
- **Expected effect:** one fewer exploratory shell round-trip and less non-decision context per planner pass.
- **Seed:** **fabro-a08a** (known, open; extend its scope to the loop lane rather than filing new).

---

**Not inspected / limits:** the live seeds tracker (401 on every query — recommendations 1, 2, 4, 5 carry new-seed justifications and should be re-deduped once the tracker is reachable); the reviewer's prompt file and PR #420's merged state (only the run-side projection was available).
