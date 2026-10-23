# Improve review — run 01M5K4BB81H36V7B4QHF9NTDS2

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: failed (5m30s wall, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 05:27+0200 by revisor `fabro_ask`

---

# Recommendations — run 01M5K4BB81H36V7B4QHF9NTDS2 (loop lane, failed in 5m30s, $0.33, 3 planner passes)

**What actually happened (from run events + conclusion):** the run died in a 3× identical `claim_check` failure cycle on seed fabro-d48f. The engine's circuit-breaker blocked the 4th attempt ("deterministic failure cycle detected … repeated 3 times (limit 3)", event seq 510) and the run ended `failed(workflow_error)` with claim-check's full 3KB stderr — the entire seed body — as the failure reason sent to #dev-fabro. Planner passes 2 and 3 (~$0.156, ~129s of the $0.33/330s) existed only to bounce. The seed body claim-check read via `seeds show` was **byte-identical (3081 bytes) in all three failures**, despite two planner journals asserting "body corrected via --set-description."

---

**1. claim-check false-blocks on paths mentioned in correction/negation prose** — highest impact; this is the run-killer.
- Evidence: the only blocking citation, `scripts/closeout-race.nu [missing_file]`, appears solely inside the body's "(Correction 2026-10-23: scripts/closeout-race.nu **no longer exists** …)" parenthetical. `claim-check.nu:120-122` blocks any rooted (`scripts` ∈ ROOTED_PREFIXES, line 68) missing_file path unless creation-intent words precede it — a *negation* mention still blocks. A corrected body that honestly says "X no longer exists" can never pass this gate.
- Change: `.fabro/workflows/develop/scripts/claim-check.nu` (plus the shared `creation-named?` window in `anchor_check.nu`): add a negation/correction window ("no longer exists", "retired", "removed", "correction:") so rooted paths inside such prose downgrade to advisory; add a case to `claim-check-smoke.nu`.
- Effect: fabro-d48f passes next run; eliminates the entire "correction note re-trips the gate" bounce class.
- Seed: **new-seed justification** — no open seed covers mention-vs-citation scoping in the claim gate (searched `claim-check`, `claim contract body correction`, `path gate`, `mention advisory` → 0 covering hits).

**2. No verify step between `--set-description` and re-claim** — the planner twice asserted a correction that never landed.
- Evidence: planner@2/@3 journals say "body corrected via --set-description" yet `seeds show` returned the identical body each time; the loop had zero feedback that the update no-oped (planner@2: 5 shell calls, 0 errors — whether the update ran isn't visible in the events I could inspect; the unchanged body is the observable fact).
- Change: `.fabro/workflows/loop/prompts/planner.md` (claim section): after any `seeds update --set-description`, mandatorily re-run `seeds show <id> --format json` and confirm the stale-path list is empty **before** routing "Seed claimed".
- Effect: a silent no-op surfaces in seconds at the planner instead of costing a planner→claim_check round trip (~60–70s, ~$0.07–0.09 each; two wasted here).
- Seed: **new-seed justification** — the run's own journal painpoint ("apply body corrections in the same pass") records the symptom but no open seed covers the verify mandate (searched `claim contract body correction`, `set-description` via `d48f` → only the claimed seed itself).

**3. Preflight counts a terminal run as in-flight** — the git-only terminality proof is develop-graph-specific.
- Evidence: preflight flagged fabro-d48f `in_flight_run=01M5K208QW…` though that run was terminal (succeeded); planner@1 burned 4 `fabro_run_search` calls (2 errors) adjudicating it. `planner-preflight.nu:114-133` proves terminality only from tip subjects like `closeout` (develop's terminal node) — loop-lane branches tip at `exit`, so they stay "in-flight" forever.
- Change: `.fabro/workflows/develop/scripts/planner-preflight.nu` `terminal-tip?` — accept the loop graph's terminal tip node, or better, land the already-specced server-state exclusion (run_search status) for arm 2.
- Effect: removes a per-pass false advisory on this queue and the planner tool-call tax of disproving it; also stops misrouting around claimable seeds.
- Seed: **fabro-997d** (open, assigned fabro: "Deterministic in-flight exclusion from server state: run_search PR link + preflight arm 2 rewrite"). Search run: `preflight in-flight` (2 hits; fabro-1437 is the orphaned-branch variant). Also folded into fabro-d48f's brief bullet 8.

**4. Terminal failure payload is a 3KB stderr blob** — run status, snapshot, and Slack all carry the mangled full seed body (visible in this run's snapshot as "…acker-guard,closeout,evidence,gate-bounce}.nu…").
- Change: `claim-check.nu` `main` — on failure emit a compact routing JSON to stdout (outcome failed, `failure_class: stale_seed_body`, blocking path list, seed id), keep the full body as a bounded tail; add `output_schema="routing"` to the `claim_check` node in loop `workflow.fabro` (it's the only guard node without it).
- Effect: run reason / Slack reads "stale body fabro-d48f: scripts/closeout-race.nu"; humans triage without opening the run; the circuit-breaker signature also becomes short and exact.
- Seed: **new-seed justification** — no open seed covers failure-payload shape (searched `circuit breaker` → only closed fabro-3d97, which is schedule pausing).

**5. Graph: give the claim-contract cycle a labeled terminal edge** (graph design / error handling).
- Evidence: tester and reviewer have `generation >= 3` deadlock edges; the `claim_check → planner` re-plan edge has no cap, so termination came from the engine-side breaker as an unlabeled `workflow_error` instead of a graph-visible state.
- Change: loop `workflow.fabro` — add `claim_check -> exit [x.kind="deadlock", condition="nodes.claim_check.generation >= 2", label="Claim contract deadlock"]`, mirroring the tester/reviewer convention (seed stays open for a human).
- Effect: same fail-fast outcome but a labeled, resumable deadlock instead of a raw workflow_error; one fewer wasted planner pass than the breaker's 3-limit.
- Seed: **new-seed justification** — no open seed covers claim-cycle capping (searched `deadlock generation cap` → 0).

**6. Pin journal array shape in the planner output contract** (prompting).
- Evidence: planner@2's first emission was rejected — "invalid JSON: key must be a string at line 8 column 624" (`"journal": {...}, ["none"]}`) — burning a repair turn; the schema explicitly leaves `context_updates` "otherwise open (journal, seed keys, …)" (`schemas/planner-output.schema.json` header).
- Change: `.fabro/workflows/loop/prompts/planner.md` output template (lines ~82–94): journal values are arrays of `{"text": "…"}` objects only, never bare arrays; mirror in develop's planner.md as part of fabro-d48f.
- Effect: removes one repair turn per claiming pass.
- Seed: **new-seed justification** — no open seed covers the planner output-contract journal shape (searched `claim contract`, `title`, `path gate` sets; nothing on output schema).

**7. Add a length guard to the RUN TITLE instruction** (tool efficiency).
- Evidence: `fabro_run_set_title` rejections — planner@2: 4 calls/3 errors, planner@3: 4 calls/3 errors (tool rejects >100 chars; the accepted title is 96). `prompts/planner.md:13` gives composition rules but no length cap.
- Change: `.fabro/workflows/loop/prompts/planner.md:13` — append "keep it under 100 characters" to the RUN TITLE instruction.
- Effect: eliminates ~6 failed tool calls per claiming run; trivial but free.
- Seed: **new-seed justification** — adjacent seeds are engine/revisor-scoped (fabro-b529 = revisor cap complaint, fabro-967e = revisor select reliability; searched `title`, 13 hits reviewed); none covers prompt-side length guidance for the loop/develop planner.

---

Not inspectable from here: the actual shell payloads of planner@2/@3 (whether `seeds update` was invoked and what it returned) — event stream exposes call counts, not command text; the byte-identical `seeds show` body across all three gates is the ground truth for recommendations 1–2.
