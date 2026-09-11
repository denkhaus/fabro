# Improve review — run 01M294HH117ZMV11HCNCEQPCDJ

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (45.4 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-11 22:11+0000 by revisor `fabro_ask`

---

All findings below come from this run's events, stage transcripts, journal, worker log, and the graph/prompt definitions in the run projection (`workflow.fabro`, `prompts/*.md`). Run shape for scale: 41.2 min wall, $2.34 LLM total — planner 106 s/$0.235, implementer 1607 s/$2.00 (85%), tester 644 s, reviewer 82 s/$0.109; zero retries, first-pass approve, PR #129.

## Recommendations, by expected impact

**1. Fix `evidence.nu` seed resolution — it captured the WRONG seed this run.**
- What happened: the evidence header read `base=14e9d5e1 seed=fabro-5082 … diff-base=bc5737fc` — fabro-5082 is a *different* in-progress seed (priority 0), and bc5737fc predates merged PRs #125–#127, so the raw diff showed unrelated `run_event/mod.rs +293` churn. The "in-progress seed spec (authoritative)" section quoted fabro-5082's body. The reviewer's transcript (event seq 787) shows ~1.9k reasoning tokens deliberating a misroute before tool-verifying the real seed and approving. Its journal painpoint says a reviewer judging only against the capture spec would have routed Changes requested.
- Change: in `.fabro/workflows/develop/workflow.fabro` give the `evidence` node `stdin_source="current_seed_id"` (the exact pattern `closeout` already uses), and make `.fabro/workflows/develop/scripts/evidence.nu` resolve spec + per-seed claim base from that id, exiting non-zero on any mismatch instead of scanning the tracker for `in_progress`.
- Expected effect: removes the wrong-spec/wrong-diff-base capture class whenever two seeds are concurrently in progress (true this run). One prevented misroute saves a full re-cycle ≈ 30 min and ~$2.4 at this run's rates, plus reviewer trust.

**2. Cut planner reconnaissance overhead (prompt: `.fabro/workflows/develop/prompts/planner.md`).**
- What happened: (a) `sd ready --assignee fabro --limit 200` returned a 200-issue firehose, 28,210 bytes, `stdout_truncated=true` — only the top High candidates mattered; (b) the planner burned **four** calls proving fabro-3ef7's body was empty (`sd show --format json`, plain, `--help`, markdown — seq 37→58); (c) ~10 sequential one-line probes (seq 63–100: rg busy_timeout, git log --grep, rg -ln Sqlite, sed fabro-db, rg projection…), each costing an LLM round trip.
- Change: add to the PROJECT_FACTS sd table one line — "JSON with no `description` key **is** the empty body; do not re-probe in other formats"; instruct the planner to batch reconnaissance probes into a single shell invocation and read only the top slice of `sd ready` (matches open seeds fabro-55a7/fabro-c3b4, re-proven by this run).
- Expected effect: planner ~106 s → ~60 s, roughly a third fewer turns/tokens; applies to every run.

**3. Pre-warm the gate toolchain image.**
- What happened: tester (`just qualitygate`) ran 643.5 s = 26% of run wall on an already-implementer-verified tree; gate output shows it rebuilds the fabro CLI renderer binary, runs workspace `fmt --check --all`, clippy and nextest across 3 crates. The graph itself documents ~15 min worst-case cold.
- Change: bake a cargo-chef/prebuilt target cache for the workspace into `fabro-toolchain:noble` (open seed fabro-cfd6), or cache `target/` across stages within a run.
- Expected effect: 4–6 min saved per develop run with zero semantic change (identical commands).

**4. Raise the reviewer's inline evidence ceiling (`reviewer` node, `workflow.fabro`).**
- What happened: the evidence capture was 49,825 bytes — over `preamble_inline_max_kb=16` — so it arrived as a blob ref (`Output (49.9 KB; full value: /tmp/fabro/runtime/blobs/…json)`) and the reviewer had to page it with tools before judging. Its context peaked at **3.4% of the 1M window**; the ceiling, not the window, is the constraint (the graph comment already says so).
- Change: `preamble_inline_max_kb=64` on the reviewer node (open seeds fabro-cf3e/fabro-8d2c propose 32; this run's capture was 49.9 KB, so 64 covers it with headroom).
- Expected effect: eliminates the blob round-trip per review and the standing "unread blob ref" rejection risk; one tool turn and ~10–20 s per review.

**5. Gate the implementer's `sd show` re-fetch on brief completeness (`prompts/implementer.md` step 1).**
- What happened: the implementer's first action (event seq 137) was `sd show fabro-3ef7 --format json | head -50` — but the seed body was empty and the planner's brief already carried the full derived spec; the call returned nothing the brief lacked.
- Change: make step 1 conditional — re-fetch only when the brief is marked incomplete; have the planner tag briefs from body-empty seeds as "brief is authoritative".
- Expected effect: 1–2 fewer implementer turns; trivial but free, and it compounds across runs.

**6. Error-handling polish surfaced by the worker log (two warns).**
- `preamble_allow_keys entry absent … key=output.gate_known_bug_hits` fired on the implementer of a **green** run, where that key legitimately doesn't exist (only produced on a red bounce). Change: downgrade to info when the producing node (`gatebounce`) wasn't visited (fabro-8275). Effect: log stops crying wolf, real envelope drift stays visible.
- PR-body generation (`zai:glm-4.7`) failed strict-JSON parsing and had to retry once without strict output before PR #129 was created. Change: make PR-body generation non-strict (or lenient schema) in the pull-request postlude. Effect: removes a retry and a failure path on every run's terminal step.

**7. Require seed bodies at intake (revisor workflow; planner journal already recommends it).**
- What happened: fabro-3ef7 was title-only; the planner authored six acceptance criteria from the title plus tree probes, so the reviewer judged planner-derived criteria, not a user spec — combined with #1, the run was one ambiguous criterion away from review ping-pong. The planner's journal observation says exactly this: "revisor intake should require a body."
- Change: intake lint in the revisor workflow rejecting/flagging body-less seeds; backfill legacy title-only seeds or split oversized ones (this one drove $2.00/26.8 min of implementer work).
- Expected effect: removes the spec-ambiguity failure mode at its source.

**8. UX: visibility during long silent stages.**
- What happened: 65% of wall time (implementer 26.8 min + gate 10.7 min) produced no operator signal; notifications fire only on `run.completed`/`run.failed` (per run settings).
- Change: a long-stage heartbeat Slack event or live per-stage progress in the web UI (open seed fabro-b769).
- Expected effect: distinguishes a healthy long run from a hung one without polling.

**Lower priority:** AGENTS.md (24.6 KB) was loaded as memory into all three agent stages (~5.8–7.1k tokens each) regardless of role relevance — per-node memory scoping (fabro-9588) would trim tokens/latency on every stage.

One caveat: I did not re-read `evidence.nu`'s source in the workspace (it is fs-hidden machinery and the run's own evidence + reviewer forensics fully document the mislabel), so recommendation #1's mechanism (`stdin_source` availability on the evidence node) is inferred from the closeout node's identical pattern rather than verified against the script's current stdin handling.
