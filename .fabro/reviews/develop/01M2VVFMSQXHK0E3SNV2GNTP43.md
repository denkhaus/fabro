# Improve review — run 01M2VVFMSQXHK0E3SNV2GNTP43

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (4.1 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 03:39+0000 by revisor `fabro_ask`

---

I inspected this run's full event history (planner/implementer/reviewer transcripts, gate output, checkpoints, timings, cost), the journal records, the tracker's open-seed listing (200 seeds, visible in the planner's `sd ready` output at event seq 38–39), and the worker warn/error log. Basis for every number below is cited. The run itself was healthy — 3m55s wall, $0.425 total, 0 retries, first-pass approval — so these are optimization recommendations, not failure post-mortems.

**Run facts used (from run events + projection):** planner 117.8s / $0.273 (64% of run cost, 15 tool calls); implementer 60.3s / $0.115 (13 shell calls, 2 errors); reviewer 22.9s / $0.037; tester gate green in 6.5s ("no crates touched"); seed fabro-d856 closed, PR #258.

## Recommendations, by expected impact

1. **Park externally-gated seeds mechanically in the preflight** (graph node `preflight`, script `.fabro/workflows/develop/scripts/planner-preflight.nu`). Evidence: the planner burned 03:31:37→03:32:37 (~60s, 8 tool calls — `sd show fabro-af22`, three rg/grep probes, a `find /` filesystem scan, a `web_fetch` of PR #784) deciding fabro-af22 is gated on unmerged upstream PR fabro-sh/fabro#784, then journaled "re-check it first in a future lap" — i.e., every subsequent run re-pays this while the High-priority seed keeps sorting to the top of `sd ready`. Change: add an `externally_gated` arm that reads a `Gate: upstream PR <url>` line from seed bodies and marks them like `in_flight`, plus one `sd update fabro-af22` adding that line now. Expected effect: −60s and ~$0.13–0.15 per develop run until #784 merges. *New-seed justification: no open seed covers mechanical parking of upstream-PR-gated seeds — fabro-9372 (seed-id projection), fabro-d9f7 (stale in_progress requeue), fabro-06e0/91ff (in-flight *runs*) all govern different exclusion sources.*

2. **Codify the tie-break rule the planner had to improvise** — seed **fabro-11d3** ("prefer seeds closing an observed failure class over same-priority polish"), file `.fabro/workflows/develop/prompts/planner.md` step 2. Evidence: after skipping af22, the planner spent seq 86–100 (~25s, `sd show fabro-a211`, two doc-locating probes, three reasoning turns) weighing a211 (docs) vs d856 (failure-class fix) before picking d856 — the exact choice fabro-11d3 prescribes. Expected effect: deterministic pick right after the af22 skip, ~25s/$0.05 saved per lap.

3. **Ship the top-N `sd ready` view** — seed **fabro-c3b4**, planner's first tracker call. Evidence: seq 38 — `sd ready --limit 200` returned 200 issues / 29,027 bytes, `stdout_truncated: true`; the planner only ever used the top 5 (the preflight candidates). Expected effect: first-turn context shrinks from ~29KB to ~2KB every planning lap, fewer truncated-output misreads.

4. **Batch planner reconnaissance into one shell call** — seed **fabro-55a7** ("sd ready + sd show + base-branch grep in one call"), `.fabro/workflows/develop/prompts/planner.md`. Evidence: 15 sequential planner tool calls, many single-purpose 60–300ms probes (seq 45, 51, 57, 63, 69, 86, 92, 98), each costing a 4–6s LLM round-trip at ~$0.014–0.02. Expected effect: ~5 fewer round-trips per lap, roughly −30s and −$0.08.

5. **Carry tool-result errors through the provider protocol** — seed **fabro-b09c**. Evidence: worker log shows 12× `unsupported_control: this provider protocol does not support the tool result error flag` during the implementer; its 2 errored shell calls (including the `ml record` failure) came back as stdout text the model had to sniff and retry from. Expected effect: errored calls surface as structured errors, killing the sniff-retry class (one retry burned this run).

6. **Put the journal digest in the PR body** — seed **fabro-1409** (per-stage cost/time table + painpoint digest), with **fabro-41b1** (non-strict PR-body JSON) as the companion fix. Evidence (UX): the planner's three decision-bearing observations (af22 skip reason, d856 body correction, c643 anchor typo) live only in `.fabro/journal/01M2VVFMSQXHK0E3SNV2GNTP43.jsonl` — invisible on PR #258; and the log shows `PR content structured generation failed; retrying once without strict JSON` at 03:35:18, exactly fabro-41b1's failure mode firing. Expected effect: the user sees why candidates were skipped and what each stage cost without grepping journals; no masked PR-body retry.

7. **Port the `rg -rn` footgun one-liner into the planner prompt** — seed **fabro-6997** (currently only in implementer.md). Evidence: seq 57 — the planner itself ran `rg -rn "find_skill_references" lib --glob '*.rs' -l`, the exact `-r n` replace-literal misparse the implementer prompt forbids; the probe returned nothing and the planner drew conclusions from an invalid command. Expected effect: eliminates invalid probes in the stage that does the most probing.

8. **Fix the `ml record` quick-reference in `AGENTS.md`** (add `--content` to the convention-type example). Evidence: implementer journal painpoint + one of its 2 shell errors — first `ml record` attempt failed with a retry hint because the flag isn't documented. Expected effect: removes a guaranteed error+retry on every implementer pass that captures a lesson (this run: mx-573c0f nearly wasn't recorded). *New-seed justification: fabro-8d81 covers mx-id printing and an implementer.md recovery line only — no seed covers the AGENTS.md example's missing `--content` flag.*

9. **Fix fabro-c643's truncated anchor path** — seed **fabro-c643** itself, one `sd update` changing `.fabro/Dockerfile.toolchai` to the real filename. Evidence: this run's preflight flagged `missing_file` for the truncated path; the planner dismissed it as "cosmetic" but it will re-flag in every future preflight until fixed. Expected effect: removes recurring anchor noise that trains planners to ignore `anchor_flags`.

10. **Demote absent optional memory files to info** — seed **fabro-a701**. Evidence: worker log — 6 ERROR lines this run (2 per agent session) for missing `/workspace/fabro/.codex/instructions.md`, a file this repo simply doesn't have. Expected effect: error-level log noise drops to zero on green runs, keeping the warn/error channel trustworthy.

**Not recommended / worked as designed:** the graph's cycle guards, claim_check (88ms), gatebounce (unused, gate was green), and the evidence pipe (7.3KB capture rendered inline, reviewer needed one shell check, $0.037) all performed well this run — no changes warranted there. One residual I could not fully attribute: the second implementer shell error beyond the `ml record` one is not distinguishable in the retained event tails; recommendation 5 would make both visible structurally.
