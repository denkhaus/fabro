# Improve review — run 01M2W5SFE33QGXKMHP5BS0068A

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (11.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 06:55+0000 by revisor `fabro_ask`

---

# Recommendations for run 01M2W5SFE33QGXKMHP5BS0068A (seed fabro-269d, 644s wall, $0.756)

Ordered by expected impact. All evidence from run events, stage journals, and the stage timings in the run conclusion; seed statuses checked against `.seeds/issues.jsonl`.

**1. Fix the prebuilt-validate fast path — `just validate-workflows develop` burned 133s of the implementer's 138s tool time.**
Evidence: implementer tool call seq 266 (`just validate-workflows develop`, timeout 600000) ran 06:37:48.7→06:40:02.0 = 133.4s, while the whole tester gate (lint-nu over 30 scripts + `cargo fmt --check --all`) took 8.2s — consistent with a cold `cargo build` of fabro-validate that fabro-af97's `FABRO_VALIDATE_PREBUILT` fast path was supposed to eliminate. Change: verify/repair the fast path in `scripts/validate-workflows.nu` and the baked `.fabro/bin/fabro-validate` in toolchain image `99c855a689b3`. Expected effect: −~2 min wall on every workflow-authoring seed (the implementer stage was 74% of run wall).
New-seed justification: fabro-af97 is closed as implemented; no open seed tracks the fast path failing inside the run sandbox.

**2. Bound the planner's two tracker firehoses.**
Evidence: `sd ready --assignee fabro --limit 200` poured 200 rows / 28,710 bytes (seq 38) and `fabro_runs_list` (no `created_since`) returned all 77 runs including this run itself (seq 80); planner conversation hit 44.5k tokens / $0.160 for a claim that needed the top of the queue plus one in-flight check. Change: `prompts/planner.md` — top-N `sd ready` view (fall back to full only when top candidates are unclaimable) and `created_since` + self-exclusion on `fabro_runs_list`. Expected effect: several KB and ~20–30s less per planning pass, every pass.
Seeds: **fabro-c3b4** (covers both halves; its 2026-09-09 extension note names `created_since` explicitly) and **fabro-6b58** (self-exclusion window).

**3. Make set-but-empty `fabro_tools=""` mean deny in the engine.**
Evidence: implementer journal painpoint + MATERIAL SEMANTIC RISK #1 — `fabro_tools=""` parses identically to unset (`graph.rs:436` split_key_list), and because the planner node's non-empty opt-in provisions services run-wide, both reviewer and implementer get the FULL fabro-run catalog registered; only the `tools=` middleware actually denies it. The seed's criterion "reviewer carries `fabro_tools=\"\"`" is satisfied as comment-backed intent, and any future node without a `tools=` list silently leaks the catalog — an ADR-0019 sharp edge the implementer paid real reasoning time to discover (340s inference total). Change: presence-sensitive parsing in `lib/foundation/fabro-types/src/graph.rs` + `lib/components/fabro-workflow/src/handler/llm/pebble.rs` (`stage_tools`). Expected effect: the graph attribute becomes mechanically load-bearing; closes a capability-leak class (a reduction, so ADR-0019-welcome).
New-seed justification: grep of the tracker finds no seed for set-but-empty semantics; the idea exists only as mulch record mx-7620f3 ("a future engine change could make set-but-empty mean deny").

**4. Include or classify the tracker churn diff in the evidence capture.**
Evidence: reviewer journal painpoint — integrity header reported `loop-churn=4 files` including `.seeds/issues.jsonl (+1/-1)` but the loop-work diff section omitted it, "the only changed loop file with no diff"; the reviewer had to guess "presumably the claim-check status flip". Change: `.fabro/workflows/develop/scripts/evidence.nu` — render that diff or annotate it `(claim-record churn)` per the reviewer's suggested one-line classifier. Expected effect: removes per-review guesswork and the risk it escalates to a Verification-blocked cycle.
Seed: **fabro-43ba** (open; its description names exactly this `.seeds/issues.jsonl +1/-1` omission — this run is fresh recurrence evidence).

**5. Carry tool errors through the provider protocol.**
Evidence: twelve `agent.warning` events in the implementer stage (seq 226…306), all `unsupported_control: "this provider protocol does not support the tool result error flag"`, plus 1 errored call in the 33-call shell series — errors currently degrade to stdout text-sniffing on this zai/glm-5.3 path. Change: engine provider-protocol error-flag support (fabro-b09c's scope). Expected effect: reliable error signaling, fewer blind retry rounds inside agent stages.
Seed: **fabro-b09c** (open).

**6. Mark upstream-blocked seeds in the preflight verdict table.**
Evidence: fabro-af22 (High, top of `sd ready`) was skipped after a full `sd show` plus a reasoning round (~06:31:47→06:32:02) re-deriving what its own body records ("Close when fabro-sh/fabro#784 merges"); the preflight marked it plain `clean` (seq 21). Every planner pass pays this tax while the upstream PR waits. Change: `.fabro/workflows/develop/scripts/planner-preflight.nu` — flag candidates whose body carries an "UPSTREAM PR OPEN / close-when-merged" status so the planner skips mechanically. Expected effect: ~20–30s + one tool call saved per pass; kills the recurring top-of-queue distraction.
New-seed justification: fabro-a285 fixed one instance (fabro-d810) by hand; no seed covers the deterministic preflight arm for the class.

**7. Raise the reviewer's inline evidence budget.**
Evidence: the evidence capture was 19,022 bytes against `preamble_inline_max_kb=16` on the reviewer node → blob-ref demotion (preview in the preamble, full value at a sandbox path); the reviewer spent its single tool call reading the blob (38.5s stage, still the cheapest review — but the detour is structural and will worsen on bigger captures; a small config seed already overflows). Change: `workflow.fabro` reviewer node `preamble_inline_max_kb` 16→32 (budget is 48KB, so no conflict). Expected effect: evidence renders inline, zero tool round-trips per review.
Seed: **fabro-cf3e** (open, asks exactly this); sibling **fabro-8cef** (plain-text blob) compounds it.

**8. Drop the now-inert `skills="discover"` from the reviewer node.**
Evidence: after this run's own change, the reviewer's `tools="read_file,grep,glob"` denies `use_skill`, yet the session still advertised 2 skills (~271 tokens, reviewer session breakdown) it can never load — the implementer's journal predicted this seam and the reviewer's observation confirmed it ("harmless but worth an eventual cleanup seed"). Change: remove `skills="discover"` from the reviewer node in `workflow.fabro` (one line), or land per-node discovery scoping. Expected effect: stops advertising unloadable skills; removes a future failed-`use_skill` attempt class.
Seed: **fabro-9588** (open, needs-triage — per-node skills/memory scoping; this run supplies the concrete reviewer case for its skills half).

Not inspected: the implementer's remaining tool-call bodies beyond the sampled events (contents were truncated at listing time) and the worker log (`fabro_run_logs`) — the timings and warnings above come from the event stream and stage journals, which were sufficient for these findings.
