# Improve review — run 01M2XV06DRXV4MS0G8GS5XVBP4

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (11.4 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 22:38+0000 by revisor `fabro_ask`

---

All evidence below is from this run's events (stage timings/cost from the run conclusion, planner transcript seq 44–100, journals from stage context updates) and from the tracker file `.seeds/issues.jsonl` (seed status checks). Run shape for scale: 670 s wall, $0.473 total — planner $0.250/123 s (53% of cost), implementer $0.166/441 s, reviewer $0.056/36 s, gate green in 43.6 s, PR #296.

## Recommendations, by expected impact

**1. Park the upstream wait-state seed out of `sd ready` — it tops the queue and re-burns half the planner every run.**
Evidence: `fabro-af22` (High) sat at the top of `sd ready`; the planner spent ~60 s / 5 LLM rounds / 5 tool calls (seq 44–76: `sd show`, a dead `git log origin/main` probe, two near-duplicate `ls-remote` probes of PR #784) re-deriving "upstream PR still open → not implementable," and journaled exactly this painpoint. Change: user parks it now (`sd update fabro-af22 --status blocked` — planner lacks that right), plus an annotation arm in `.fabro/workflows/develop/scripts/planner-preflight.nu` that flags seed bodies carrying "UPSTREAM PR OPEN" so the verdict table answers the wait-state question deterministically. Expected effect: ~$0.10–0.15 and ~60 s saved per develop run for as long as af22 leads the queue. **New-seed justification:** tracker grep shows no open seed covers parking upstream-blocked wait-state seeds (fabro-32db is in-flight semantics, fabro-c3b4 is listing volume, fabro-af22 itself is the upstream fix).

**2. Fix verify.nu's blind spot for inline `#[cfg(test)]` test edits — seed fabro-2f70 (open).**
Evidence: the implementer's own journal this run: verify.nu "classified this diff as code-touched only (test-file-touched empty) even though an inline #[cfg(test)] mod tests was edited… ran compile-check instead of the crate suite." The new table-driven test ran only because the implementer happened to run focused tests manually. Change: `scripts/verify.nu` `is-test-file` (~line 24) also flags crates whose diff touches `#[cfg(test)]` regions. Expected effect: the implementer pre-gate actually executes newly written inline tests — the project's stated convention — instead of silently degrading to compile-check.

**3. Sweep non-blocking reviewer findings into seeds at closeout — seed fabro-22fa (open).**
Evidence: the reviewer found a real latent defect this run — "octal \nnn escapes decode byte-per-char (Latin-1), so multi-byte UTF-8 quoted paths mis-decode" in the just-merged `decode_c_quoted_path` — and marked it non-blocking; closeout closed fabro-c0a2 at 22:12:30, so that finding now lives only in `.fabro/journal/01M2XV06DRXV4MS0G8GS5XVBP4.jsonl`. Change: extend the re-file mechanism in `.fabro/workflows/develop/scripts/closeout.nu` to file reviewer-journal non-blocking observations as open seeds before `sd close`. Expected effect: this exact decode defect (and every future non-blocking finding) becomes actionable work instead of dying with the closed seed.

**4. Make the preflight's in-flight arm honor terminal runs — recurrence of seed fabro-32db (closed 2026-09-19, acceptance now contradicted).**
Evidence: this run's preflight table (22:01:25) marked `fabro-af22`/`fabro-b03f` `in_flight: true` under run `01M2XSA56XFSQPH3PAM4KXZCGY`, which had already **failed** (terminal) at 21:39:32 — despite the script's own header comment and fabro-32db's definition ("in flight ONLY while its run is non-terminal"). The planner then burned a long deliberation (seq 52 reasoning) resolving the contradiction before proceeding. Change: the `in-flight-claims` arm in `.fabro/workflows/develop/scripts/planner-preflight.nu` must exclude terminal-run branches (or annotate `in_flight_run` status) — reopen fabro-32db or file a successor with this run as recurrence evidence. Expected effect: preflight table matches its documented semantics; the planner stops adjudicating terminality, and b03f-type candidates aren't falsely shadowed.

**5. Top-N `sd ready` view in the planner — seed fabro-c3b4 (open).**
Evidence: the planner's first tracker call returned **200 ready issues / 28,468 bytes** (event 48) — roughly 4k tokens of firehose to learn which 5 seeds are candidates (the preflight table already names them). Change: `.fabro/workflows/develop/prompts/planner.md` — pick from a top-N view (`sd ready --first 10`), full listing only when the top candidates are unclaimable. Expected effect: smaller planner context and fewer deliberation rounds per pass; composes with #1.

**6. Bound the `fabro_runs_list` in-flight call — seed fabro-6b58 (open); structural fix fabro-9372 (open).**
Evidence: the planner called `fabro_runs_list` unbounded and got **95 runs**, nearly all terminal with byte-identical generic goals, then had to map runs→seeds indirectly (this is exactly what fabro-9372 documents). Change: `planner.md` step 4 — pass `created_since ≈ 48h` and exclude the self-run id; engine-side, expose `current_seed_id` in the runs-list projection (fabro-9372). Expected effect: tool output drops from 95 rows to the 2–3 that matter; the goal-prose/journal seed-id heuristic disappears.

**7. Carry tool-result errors through the provider protocol — seed fabro-b09c (open).**
Evidence: three `agent.warning: unsupported_control — "this provider protocol does not support the tool result error flag"` in this run's planner (seq 82, 89, 96), each following an expected non-zero shell exit (a `grep -l` no-match chain, exit 1) — the error signal degraded to stdout text-sniffing and the model had to interpret ambiguity. Change: propagate the tool error flag in the agent-session/provider layer (`lib/components/fabro-workflow` agent session / pebble Environment protocol), or emit an explicit failure marker. Expected effect: deterministic error signaling for failed/non-zero tool calls; expected no-match exits stop masquerading as possible failures.

**8. Batch planner reconnaissance into one chained shell call — seed fabro-55a7 (open).**
Evidence: 10 sequential planner shell calls, including two near-duplicate `ls-remote` probes of PR #784 (seq 65, 71) and the failed `origin/main` probe (seq 59) — each costing a full LLM round-trip (~5–9 s TTFT each). Change: `planner.md` — chain the candidate sweep (`sd show` of top candidates + upstream-ref checks) into one labeled shell call. Expected effect: ~3–5 fewer round-trips per planning pass (~30–50 s), per the seed's own measured basis and this run's call pattern.

**What needed no change:** the evidence pipe worked as designed this run — the 8.2 KB capture rendered inline for the reviewer (under the 16 KB `preamble_inline_max_kb` cap, no blob detour), the gate ran warm (43.6 s), and review approved first-pass in 36 s with 4 focused `read_file` calls. No recommendations there.
