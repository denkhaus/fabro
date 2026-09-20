# Improve review — run 01M2YSY6EGPS2G5ZG8FS3AG0VZ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (7.3 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-20 07:15+0000 by revisor `fabro_ask`

---

## Run recap (what the evidence shows)

Clean single-lap run: `tracker_guard → preflight → planner → claim_check → implementer → tester (gate green) → evidence → reviewer (approved) → closeout`. Claimed and closed seed **fabro-866a** (2-insertion prompt edit to `.fabro/workflows/develop/prompts/implementer.md`). Total: **424 s wall, $0.448** — planner alone was **210 s / $0.315 (≈70% of cost, 50% of wall)** for a 2-line markdown seed (from run conclusion/stage timings). All friction this run concentrated in the planner lap and in recurring tracker triage. Recommendations, ordered by expected impact:

---

**1. Stop the planner reverse-engineering in-flight seed ids — implement open seed `fabro-9372`.**
- What happened: the planner's `fabro_runs_list` call returned 112 runs, most with `pull_request.state: null`, so it burned ~3 LLM rounds and 2 shell calls grepping `.fabro/journal/<run>.jsonl` files of five recent runs to recover their claimed seed ids (run events seq 55–73, 07:02:33→07:03:22, ≈50 s of the 210 s stage) — exactly the workaround `fabro-9372` exists to remove.
- Change: expose `current_seed_id` in the `fabro_runs_list` projection (engine run-catalog, per the seed); no prompt change needed — the planner prompt already tells it to recover ids from goal/journal "until the projection carries them."
- Expected effect: −2 to −4 planner LLM rounds per run (~30–60 s, ~$0.04), and the null-PR-state ambiguity (PRs 318/314/308 all `state: null` this run) stops needing adjudication.

**2. Park the two upstream-blocked seeds so `sd ready` stops re-surfacing them — apply the `fabro-a285` mechanism to `fabro-af22` and `fabro-23d4`.**
- What happened: the planner skipped priority-1 `fabro-af22` **again** (journal: "skipped again … recurring friction") after a full `sd show` + reasoning pass, and re-derived `fabro-23d4`'s external-CLI infeasibility (seq 59–91) that run 01M2YR7PA6 had already journaled. `sd ready` lists both at the top every run because they carry zero blockers.
- Change: mirror open seed `fabro-a285` (same fix class, scoped to fabro-d810): file a tiny "upstream: fabro-sh#784 merge" / "upstream: seeds note command" tracking seed and `sd block fabro-af22 <id>` / `sd block fabro-23d4 <id>` — `sd ready` only lists unblocked seeds, so both drop out of triage until unblocked. (a285 covers the mechanism; af22/23d4 are new instances needing the same treatment — one sibling seed or an a285 generalization.)
- Expected effect: −1 to −2 planner rounds every run until upstream lands; kills the recurring "skipped again" journal noise and the mis-triage risk from af22's misleading title.

**3. Replace the 200-seed `sd ready` firehose with a top-N view — implement open seed `fabro-c3b4`.**
- What happened: `sd ready --assignee fabro --limit 200` returned **200 seeds / 28 KB** (`stdout_truncated: true`, run event seq 46); the planner consumed it all into context (conversation bucket hit 58.6 k tokens) but only ever used the top 5 — which the preflight node had already verdict-tabled.
- Change: per `fabro-c3b4`, make the planner's first call a top-N view (the PROJECT_FACTS command table in `.fabro/workflows/develop/prompts/planner.md` is the edit site); fall through to the full list only when the top-N is exhausted.
- Expected effect: ~28 KB less first-round context, faster candidate triage, cheaper every planner lap.

**4. Make seed-body corrections mechanical — new seed (planner.md step 3).**
- What happened: re-emitting fabro-23d4's body under `sd update --description`, the planner **corrupted the Basis hash** on the first attempt (`1e95a3bfe93d5d70d9b6aa6a844359b946278278`, event seq 83), caught it, and burned a full extra LLM round + rewrite (seq 88–91) — the exact failure mode fabro-23d4 documents, now observed live a second way.
- Change: in `.fabro/workflows/develop/prompts/planner.md` step 3 (intermediate case), mandate a mechanical re-emit: `sd show <id> --format json | jq -r .issue.description > /tmp/body` → append the correction via heredoc → `sd update <id> --description "$(cat /tmp/body)"` → one grep assert that the `Basis:` line survived byte-identical.
- Expected effect: eliminates the hand-retyping corruption class (silent tracker-history loss) and the ~25 s / $0.04 recovery round.
- New-seed justification: fabro-23d4 covers the upstream `sd note` command (external CLI, not claimable) and closed fabro-c0a8 covered only the when-to-correct rule — no open seed covers the planner-side safe re-emit procedure.

**5. Give the journal contract schema teeth — extend open seed `fabro-017f`.**
- What happened: the planner's first final JSON was invalid (`journal.observations` carried `["none"] as any`, event seq 118); the engine burned one output retry and the model re-emitted 27 s later (seq 123, retry cost $0.0225).
- Change: in `.fabro/workflows/develop/schemas/planner-output.schema.json`, constrain `context_updates.journal` (`painpoints`/`observations` as `{type:"array", items:{type:"string"}}`) — fabro-017f already demands journal keys in the schema; this adds value-shape so malformed payloads fail mechanically with a precise validation error instead of a generic parse-retry.
- Expected effect: malformed-journal retries either vanish (model sees the exact constraint) or fail fast with a named field — saving ~25–30 s on the laps that hit it.

**6. Bound pre-first-token stalls — prioritize open seed `fabro-e395`.**
- What happened: eight planner LLM rounds each stalled 5–14 s before first token (llm.started→first_output pairs, seq 42/48/56/62/74/80/92/98) — roughly 60 s of the planner's 193 s inference time was pure TTFT wait; none errored, so no retry could fire.
- Change: per `fabro-e395`, per-call TTFT timeout with transparent retry in the stage LLM client (`lib/components/fabro-workflow/src/model_fallback.rs` area). It's P2, unblocked, and preflight-clean this run — a natural next claim.
- Expected effect: recovers a large share of ~60 s/run here, and bounds the 163 s tail the seed was filed on.

**7. Cap implementer.md step 4 — implement open seed `fabro-7b2a`.**
- What happened: this run's diff inserted *another* sentence into the already ~700-word single-paragraph step 4 (visible in the implementer diff hunks); the implementer prompt ran 25 k input tokens, planner 70 k. Every future implementer pass re-reads the grown paragraph.
- Change: per `fabro-7b2a`, cap step 4's operative rule and move the accumulated evidence citations (measured-once war stories) to footnotes in `.fabro/workflows/develop/prompts/implementer.md`.
- Expected effect: smaller implementer preambles on every pass — compounding token/latency savings across the loop.

**8. Qualify the PROJECT_FACTS fs_hide bullet per node — new seed (project-facts.md).**
- What happened: the reviewer's `read_file`/`grep` on `.fabro/workflows/develop/prompts/implementer.md` **succeeded** despite PROJECT_FACTS saying `.fabro/` is fs_hide-blocked for file tools — because the reviewer node deliberately carries no `fs_hide` (graph comment). The reviewer spent a reasoning detour and a journal observation on the apparent contradiction (seq 232–239).
- Change: one clause in `.fabro/workflows/develop/prompts/project-facts.md`'s loop-asset bullet: "hidden on planner/implementer; the reviewer node deliberately carries no fs_hide so it can see every diff."
- Expected effect: no more confusion detours on loop-asset reviews.
- New-seed justification: open fabro-a512 targets disproved fs_hide claims in `workflow.fabro` comments and open fabro-b7ab targets `docs/` codification — neither edits project-facts.md's per-node wording.

**9. Silence by-design prompt-lint warnings in the gate — new seed (prompt-lint.nu).**
- What happened: the gate output carried 11 warnings every run (run event seq 205 / tester output): the stale date-pin warn plus 10 "routing-named property" warns on `planner-output.schema.json` and `develop-output.schema.json` — fields those schemas **must** carry (the fabro-017f teeth pattern requires routing fields). This noise rides into every reviewer preamble via the tester section.
- Change: in `.fabro/scripts/prompt-lint.nu`, allowlist routing-named top-level fields in `@schemas/*.json` (intentional contracts) and downgrade to info.
- Expected effect: gate log and reviewer preambles carry signal only; ~10 lines of noise removed per run.
- New-seed justification: closed fabro-a211 only documented the routing-field contract; open fabro-8275 covers a different engine warn (preamble allow-key absence) — no seed covers the prompt-lint schema-warn suppression.

---

**What already worked (no change needed):** the just-landed fabro-866a pattern was validated live this run — implementer recon ran as one 618 ms chained shell round (implementer journal observation), the tester gate correctly no-oped on a loop-asset-only diff (25 s, "no crates touched"), and closeout closed the seed deterministically in 0.4 s. Sources: run events (seq 35–239), stage timings/usage from the run conclusion, tester gate output, and the three stage journals — all for run 01M2YSY6EGPS2G5ZG8FS3AG0VZ.
