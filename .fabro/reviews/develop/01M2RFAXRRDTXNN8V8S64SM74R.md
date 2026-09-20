# Improve review — run 01M2RFAXRRDTXNN8V8S64SM74R

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (23.1 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-17 20:30+0000 by revisor `fabro_ask`

---

## Run basis (from run events, stage outcomes, worker log, and stage journals)

Run 01M2RFAXRRDTXNN8V8S64SM74R claimed **fabro-9ec3** and shipped it green in 23.1 min (20:01–20:24, PR #233): preflight 1.1 s / $0, planner 51 s / $0.101, **implementer 1162 s / $2.756 (~93 % of agent cost; 407k input tokens, 91 shell calls, conversation peaked at 391k tokens)**, tester gate 6.3 s (lite path, "no crates touched"), evidence 44 KB, reviewer ~1.5 min, closeout closed the seed. The new preflight→schema machinery itself worked exactly as designed (1.1 s verdict table replaced LLM probing; dup-run-check `clean`; both fixture batteries gate-wired). The recommendations below target what actually hurt.

## Recommendations, ordered by expected impact

1. **Strip the fabro run-tool registry from the implementer node** — set `tools=` allow-list on the `implementer` node in `.fabro/workflows/develop/workflow.fabro`. At 20:20:49 the implementer called `fabro_run_interact {get}` on **its own run**; the tool returned "original token count: 947,879" (2.7 MB truncated), the next LLM call's input jumped 114k→288,907 tokens and cost **$0.441 in one round** — the model itself journaled "huge unnecessary call". Node: `implementer`. **Seed: fabro-269d** (open, needs-user; also covers the reviewer, whose session also carried the full mutation-capable registry per event seq 735). Effect: this $0.44/run waste class becomes mechanically impossible, ~3–5k prompt tokens drop from every one of ~90 rounds, and ADR-0019 capability-delta risk shrinks. Complements open **fabro-c00c** (prompt-side self-introspection ban — extend it to `fabro_run_interact get`).

2. **Make the routing-JSON retry class repairable instead of retrying** — per fabro-270c, make `journal` payloads shape-symmetric and add engine-side repair of trivial JSON errors before spending an output retry. Evidence: the implementer's first final answer (seq 691) died on an invalid `\s` escape inside journal text plus a malformed journal object; the engine rejected it at seq 699 ("expected `,` or `}` at line 5 column 878") and the fix burned **two full-context rounds at ~400k input each** (20:20:49→20:21:40). File: `.fabro/workflows/develop/prompts/implementer.md` (journal shape) + `lib/components/fabro-workflow/src/handler/structured_output.rs` (repair). **Seed: fabro-270c** (open). Effect: eliminates 1–2 full-context retry rounds per malformed emission (~$0.10–0.60 this run).

3. **Vendor the nushell-gotchas as a skill the implementer loads on `.nu` seeds** — create `.fabro/skills/nushell-scripts/` + one line in `implementer.md` step 2. Evidence: the implementer's own journal painpoint this run — `do { <nu-def> } | complete` fails at runtime, and `git fetch` refspec wildcards map only the tail segment — both discovered by trial inside the stage consuming 93 % of run cost (6 shell errors across the pass). **Seed: fabro-d19e** (open). Effect: skips the probe/debug cycles on every script seed; this run's class recurred in the immediately preceding run too.

4. **Chain implementer recon and verification into single shell calls** — fold the target-file read into the dup-run-check call and `git diff --stat` into the `just verify implementer` call in `implementer.md` step 1/4. Evidence: 91 shell calls across ~89 LLM rounds, including full-file `cat`s of `planner-preflight.nu` (11.2 KB) and `planner.md` (21.4 KB) and separate pure-read rounds (e.g. seq 86–88). **Seed: fabro-866a** (open). Effect: 2–4 fewer rounds per implementer pass (~$0.05–0.15, more on loop-asset seeds where this run spent everything).

5. **Render binary-classified diffs as text in the evidence capture** — `evidence.nu` falls back to `git diff --text` under existing size bounds. Evidence: `planner-preflight-anchor-fixtures.nu` rendered "Binary files differ"; the implementer journaled the friction, and the reviewer then burned **3 of its ~5 shell calls** (od, NUL-grep, `git diff --text`) adjudicating that one UNSEEN item (seq 750–759). File: `.fabro/workflows/develop/scripts/evidence.nu`. **Seed: fabro-533c** (open, P1, fresh — same file, same NUL-byte fixture). Effect: removes the per-review anomaly investigation and the Verification-blocked escalation risk.

6. **Cap loop-churn diffs in the evidence capture** — numstat + first ~30 changed lines per churn file. Evidence: the 44 KB capture was blob-ref'd ("Output (43.9 KB …)") past the reviewer's 16 KB inline ceiling, and the bulk is churn: `.seeds/issues.jsonl` +533/−533 whole-file rewrite and the `.mulch` record. File: `evidence.nu`. **Seed: fabro-020b** (open; sibling budget-side seed fabro-cf3e asks ≥40 KB inline). Effect: typical captures land inline; one fewer blob round-trip per review.

7. **Enforce newline-bulleted briefs** — planner-side mandate + engine render. Evidence: despite planner.md step 6's "BULLETED acceptance criteria", `current_seed_brief` shipped as one ~2.9 KB paragraph with inline (1)–(6); implementer and reviewer both re-parsed criteria mush. Files: `prompts/planner.md` step 6, engine preamble renderer. **Seeds: fabro-9e49** (open, prompt side) and **fabro-260c** (open, renderer side). Effect: cheaper, misparse-resistant criteria for the two downstream stages every run.

8. **Default PR-title/body generation to non-strict JSON** — worker log 20:24:07: "PR content structured generation failed; retrying once without strict JSON output" — the guaranteed failed first pass fired again on this run's publish. File: `lib/components/fabro-workflow/src/pipeline/pull_request.rs`. **Seed: fabro-41b1** (open). Effect: one fewer failed LLM call and ~10–15 s off every run's terminal step.

9. **Fix the pipeline-progress denominator** — the implementer header read "1 of 8" with preflight+planner done, the reviewer "3 of 8" with five non-meta stages done (both stage prompts, seq 73/730). **Seed: fabro-9e8b** (open; this is third-run counter-evidence post-#175). Effect: honest mid-run loop-state for agents and the UI.

10. **Restore the WARN/ERROR channels** — this run's worker log: ~166 of 170 warn lines are the duplicated pebble `unsupported_control` warning (fabro-9a42), plus 2 ERROR lines for the expected-absent `.codex/instructions.md` at reviewer session init, 20:21:54 (fabro-a701, sibling fabro-5c45). **Seeds: fabro-9a42** (demote/dedup the warn) and **fabro-a701** (absent optional file → info). Effect: log sweeps surface real failures; on green runs today they surface nothing but noise.

**Not recommended from this run:** the tester gate (6.3 s lite path — fabro-01b working as intended), the preflight handoff (1.1 s, zero tokens), and the dup-run-check/fixture battery all performed exactly as their seeds promised; no change warranted on that evidence.

One scope note: I could not inspect the reviewer's final verdict event or the closeout transcript tail directly (event listing truncated at seq 760), but the run-level status `succeeded(completed)` with the closeout having run implies the Approved→close path fired; recommendations 5, 6, and 9 rest on the reviewer's observed tool calls and prompts, which I did inspect.
