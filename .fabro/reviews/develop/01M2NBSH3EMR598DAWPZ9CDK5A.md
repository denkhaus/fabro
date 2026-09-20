# Improve review — run 01M2NBSH3EMR598DAWPZ9CDK5A

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (10.7 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-16 15:16+0000 by revisor `fabro_ask`

---

All evidence below is from run events (stage projections, journal, tool transcripts) of run `01M2NBSH3EMR598DAWPZ9CDK5A` — seed `fabro-cf2a`, all 6 stages first-pass green, 10m25s wall, $0.533 total. Cost split: implementer $0.242/45%, planner $0.232/43%, reviewer $0.060; the four deterministic nodes (tester 3.7s, evidence 0.3s, closeout 0.35s, gate-bounce unused) cost ~4.3s combined — so every recommendation below targets the three agent stages.

## Recommendations, ordered by expected impact

**1. Stop the implementer from re-deriving Nushell 0.115 semantics — seed `fabro-d19e` (open).**
What happened: the implementer burned 394.9s inference against 7.1s of tool time to write ~50 lines of nu in `.fabro/scripts/dup-run-check.nu`, recording 3 shell errors. Two distinct pitfalls — `mut x = null` type-inference (`nu::shell::type_mismatch`) and the invalid `` $x.field? `` optional-path form — were hit and re-fixed, then re-recorded as `mx-a68f5b`. The previous run (`01M2N8B4QY`) recorded the *same* class as `mx-05fb4e`; nothing fed it back into context.
Change: land the nushell-scripts skill per `fabro-d19e` and reference it from `.fabro/workflows/develop/prompts/implementer.md` so the two recorded pitfalls ride in via `ml prime`/skill instead of rediscovery.
Expected effect: removes roughly half of this pass's inference (~$0.10–0.12, ~3 min per nu-heavy seed) and eliminates the recurring error-retry loop; this is the second consecutive run paying it.

**2. Batch planner reconnaissance and cap the `sd ready` firehose — seeds `fabro-55a7` + `fabro-c3b4` (both open).**
What happened: the planner ran `sd ready --assignee fabro --limit 200`, which returned a 28,117-byte listing of all 200 open seeds, then issued ~9 more *sequential* small probes (greps on closeout wiring, `sd sync --help`, `cat workflow.toml`, an 11KB `cat closeout.nu`) to verify the seed basis — 18 tool calls total, 152.2s inference, $0.232 = 43% of run cost for a claim + brief.
Change: per `fabro-55a7`, one labeled compound shell call for recon (sd show + wiring greps together); per `fabro-c3b4`, a top-N `sd ready` view in `.fabro/workflows/develop/prompts/planner.md` so the 28KB listing stops entering context when only the top High candidates matter.
Expected effect: fewer planner turns and a much smaller conversation block (33.3k of this stage's 44k input tokens were conversation); realistic saving is 30–60s and ~$0.05–0.10 per planning pass.

**3. Put per-criterion check outputs into the evidence capture — seed `fabro-d89a` (open).**
What happened: the reviewer's own journal says it "independently re-ran the seed's reproduction command and the foreign spot-check" — 4 shell calls re-executing *exactly* the assertions the implementer had already reported PASS with commands in `implementation_summary`. 43.4s / $0.060 review spent duplicating green checks.
Change: per `fabro-d89a`, extend `.fabro/workflows/develop/scripts/evidence.nu` to carry the implementer's per-criterion check outputs (command + result) in the capture, so the reviewer's "verification economy" clause has captured evidence to judge from.
Expected effect: cuts the reviewer's live re-run detour on clean runs — most of its 4 tool calls and a meaningful slice of its 43s; also removes the only path by which a reviewer-side flake (see rec 4) could corrupt an otherwise approved review.

**4. Forbid chaining a script's write and its verification run in one shell call — NEW SEED required.**
What happened: the implementer's journal observation: after the mut-to-let fix, "the first full re-run printed closure foreign with the inherited trailer already populated; an immediate re-run with identical input printed closure self. Likely the first execution raced the file write (command chaining in one shell call)." That is a nondeterministic verdict from the very tool this seed hardens — the worst possible failure mode for a determinism check.
Check done: no open seed covers the write→execute race within a single chained call — `fabro-4601` covers *parallel batch edits* (a different mechanism), `fabro-50f8` covers null-path exercises. Justification: it is a distinct transient-verdict class, observed live this run, and cheap to kill at the prompt level.
Change: one line in `.fabro/workflows/develop/prompts/implementer.md` (step 2/4 area): the write of a script and any execution of that same script must be separate shell calls.
Expected effect: eliminates flaky PASS/FAIL verdicts on edited scripts — the class that produced this run's one unexplained observation.

**5. Expose `current_seed_id` in the `fabro_runs_list` projection — seed `fabro-9372` (open), plus `fabro-6b58` for bounding.**
What happened: the develop goal is the generic string with no seed id, so during the in-flight check the planner had to reason "only the current run is non-terminal (this run itself — self-exclusion)" across 25 returned runs, and the prompt carries a whole journal-grep fallback procedure for recovering other runs' seed ids.
Change: per `fabro-9372`, add `current_seed_id` to the runs_list payload server-side and let planner.md step 4 read it directly; `fabro-6b58`'s `created_since` bound shrinks the 25-run scan this run did.
Expected effect: the in-flight guard becomes mechanical — one reasoning turn saved every planner pass, and the journal-grep fallback path (unexercised this run only because no other run was in flight) stops being load-bearing.

**6. Split the PROJECT_FACTS sd-command table per role — seed `fabro-52b4` (open).**
What happened: the rendered implementer and reviewer prompts both carry the *full* sd table — including the claim form and the `sd close` rules — while the implementer prompt simultaneously says "Do NOT claim, close, or re-status seeds." The memory block cost ~6.5k tokens per stage (planner, implementer, reviewer each), and the reviewer got tracker mechanics it must never use.
Change: per `fabro-52b4`, split the table in `.fabro/workflows/develop/prompts/project-facts.md` into per-role includes (planner full; implementer `sd show` only; reviewer none).
Expected effect: smaller, role-accurate preambles on all three agent stages — direct input-token reduction every cycle and one less contradiction class for the implementer to reconcile.

**7. Adopt `--self <run-id>` in dup-run-check callers — seed `fabro-4b76` (open, priority 1).**
What happened: the implementer prompt actually executed in this run still says `nu .fabro/scripts/dup-run-check.nu <current_seed_id>` with no `--self` (visible in the rendered `implementer@1` stage prompt) — so the closure-identity machinery this run just extended in `dup-run-check.nu` remains dead code at its primary call site, exactly as `fabro-4b76` describes. The brief correctly waived it as out of scope; the seed stays the necessary follow-up.
Change: per `fabro-4b76`, update implementer.md step 1 (and revisor analyze Step 3.5) to call with `--self <run-id>`, and check in the fixture battery instead of improvised `/tmp` sd wrappers.
Expected effect: `inherited-trailer`/`closure_note` actually fire in production preflights; without this, this run's 60-line fix is only exercised by hand.

**Not recommended now (measured healthy in this run):** `preamble_inline_max_kb`/blob handling — the evidence capture came in at 10.2KB, under the 16KB cap, and rendered inline with no blob round-trip; the 48KB graph budget held. Leave as-is until a capture actually trips it.

Not inspectable: per-shell-call latency inside the implementer's 3 errored calls (event payloads retain only aggregate tool stats plus journal narrative), so the savings estimate for rec 1 is inferred from inference-time deltas between error-fix turns, not from timed failures.
