# Improve review — run 01M2W97M3ZYPKJHDPENVKXD7E9

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (9.5 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 07:46+0000 by revisor `fabro_ask`

---

All evidence below is from this run's stage transcripts, journals, conclusion timings, and the worker log (run 01M2W97M3ZYPKJHDPENVKXD7E9, seed fabro-e907, 9/9 stages green, zero retries, first-pass approval, $0.598 total, 9 m 30 s wall). Cost split: planner $0.412 (69%), implementer $0.141, reviewer $0.045. The healthy pipeline shape is not the problem — the planner lap is.

## Recommendations (by expected impact)

**1. Detect externally-blocked seeds before the planner LLM lap — new seed required.**
What happened: the top `sd ready` seed, fabro-af22 (High), states in its own body "UPSTREAM PR OPEN: fabro-sh/fabro#784 … Close when #784 is merged upstream." The planner read that at 07:32:15 (seq 46) but still ran 9 shell probes and 9 LLM round trips — including `rg` searches for `find_skill_references`, `~/.cargo` checkout hunts, a failed `git fetch origin main`, and `git remote -v` — before concluding "externally in-flight, skip" at 07:33:28 (seq 104). That's ~73 s and ~$0.154 of LLM cost (sum of per-message usage, seq 50–104) = ~26% of the entire run, spent re-deriving line 1 of the seed body. And it recurs: af22 stays top-of-ready until #784 merges, so every develop run pays this again.
Change: add an `externally_gated` arm to `nu .fabro/workflows/develop/scripts/planner-preflight.nu` (parse candidate seed bodies for an open-upstream-PR Status section; mark it in the verdict table the planner already trusts) plus one line in `.fabro/workflows/develop/prompts/planner.md` step 3: a seed whose body names an open external PR is journaled-skip, never tree-probed.
Expected effect: removes ~$0.15 and ~75 s per run while af22 sits at the top; planner laps start at candidate #2.
New-seed justification: the planner's own journal painpoint from this run proposes exactly this and names no owner; fabro-06e0/fabro-91ff/fabro-9372 exclude only *this repo's* in-flight runs/PRs — no open seed covers externally-gated (upstream-repo) seeds.

**2. Close absorbed sibling seeds at closeout — fabro-ab38 (open).**
What happened: the implementer's summary and the reviewer's journal both record "fabro-6ab7's conductor-inspects criterion is satisfied by this change" (the revisor `inspects` extension). Closeout, pinned to `stdin_source=current_seed_id`, closed only fabro-e907; fabro-6ab7 remains open and claimable (verified in the closeout diff). The absorption note traveled implementer → reviewer journal → nowhere; no node owns it.
Change: implement fabro-ab38 in `nu .fabro/workflows/develop/scripts/closeout.nu` — sweep seed ids cited in the run diff/journal and superseded-close satisfied siblings with closure notes.
Expected effect: prevents a future run claiming 6ab7 and burning a full cycle (~$0.6 / ~10 min) on already-landed work; today it's only saved by a probabilistic base-history grep.

**3. Cut the planner's context firehose — fabro-c3b4 (open) + fabro-6b58 (open).**
What happened: the planner's first two probes pulled `sd ready --limit 200` (200 seeds, 28.7 KB, seq 38) and `fabro_runs_list` (79 runs, 10.6 s, seq 41) — yet the top-5 candidate verdict table was already inline in `output.preflight`. Planner input: 62,247 tokens vs implementer 24,871 and reviewer 17,861.
Change: per fabro-c3b4, use a top-N `sd ready` view (or trust the preflight table first, `sd show` only the adjudicated candidate); per fabro-6b58, bound `fabro_runs_list` with `created_since` + self-exclusion.
Expected effect: shrinks the costliest stage's input base and the surface area for af22-class detours; planner is 69% of run cost, so token cuts land directly there.

**4. Port the `rg -rn` footgun line to the planner prompt — fabro-6997 (open).**
What happened: at seq 57 the planner ran `rg -rn "find_skill_references"` — `-r n` means replace-with-literal-"n", so it silently returned nothing, which fed the wrong hypothesis "maybe it lives in the pebble-coding-agent dependency" (seq 62 reasoning) and extended the af22 detour. The exact rule already exists in `implementer.md` ("rg flag discipline") but not in `planner.md`.
Change: land fabro-6997's two one-liners in `.fabro/workflows/develop/prompts/planner.md` (never `rg -rn`; hidden-path glob returns empty).
Expected effect: eliminates a silent-wrong-result probe class in the stage that does the most probing.

**5. Fix evidence truncation for loop-asset seeds — fabro-cf3e (open), extended with a line-cap arm.**
What happened: the reviewer's preamble rendered the evidence capture with "(52 lines omitted)" (8.8 KB capture vs the 200-line `preamble_output_max_lines` sized by fabro-meta-c9f2 for ~120-line captures). The reviewer compensated with 4 read-tool calls (fine here at $0.045), but the same omission on a seed whose loop-work diff *is* the review scope is the documented escalation path to "Verification blocked."
Change: reviewer node in `.fabro/workflows/develop/workflow.fabro` — land fabro-cf3e's `preamble_inline_max_kb` 16→32 *and* raise `preamble_output_max_lines` 200→~300 in the same change (cf3e covers the KB knob only; the line cap is what bit this run — add it as a second arm).
Expected effect: reviewer judges from the capture directly; removes per-review tool detours and the blocked-re-capture risk on loop-asset seeds.

**6. Mechanize cross-seed premise checks — fabro-3839 (open).**
What happened: the planner spent 5 probes / 27 s (seq 105–128) discovering fabro-d9f7 "extends fabro-0da8's tracker-guard node" while fabro-0da8 is still open — then only journaled it. Per this workflow's own rule ("a requirement that lives only in a journal entry does not exist"), the next planner re-derives the same dead end, and d9f7's body was never corrected via `sd update --description`.
Change: implement fabro-3839 (lint filed seeds for resolvable refs) covering seed-to-seed references: flag bodies that name another seed's artifact when that seed is still open.
Expected effect: the d9f7-class contradiction is flagged by the preflight table (anchors already are) instead of re-derived per run.

**7. Land the three filed log-hygiene seeds this run re-evidenced — fabro-a701, fabro-41b1, fabro-b09c (all open).**
What happened in the worker log: 6 ERROR lines for expected-absent `.codex/instructions.md` across all three agent sessions (fabro-a701); "PR content structured generation failed; retrying once without strict JSON" at 07:40:58 (fabro-41b1); 16× "provider protocol does not support the tool result error flag" during the implementer, which had 1 shell error (fabro-b09c — the error signal degraded to stdout sniffing exactly as filed). There's also a recurring benign `context_update_dropped: output.planner` WARN each planner pass.
Change: land the three one-line/one-scope fixes as filed.
Expected effect: clean warn-level log signal so real failures (like the PR-postlude retry) aren't buried; error-bearing tool results reach the model reliably.

**8. Correct the PROJECT_FACTS fs_hide claim for the reviewer envelope — new seed required.**
What happened: the reviewer's journal records that `read_file` on `scripts/validate-workflows.nu` and `.fabro/workflows/*` *succeeded* while PROJECT_FACTS (rendered into `reviewer.md`) says those paths are file-tool-blocked — the reviewer node deliberately carries no `fs_hide`. The prose is role-inaccurate; a future reviewer trusting it could route "Verification blocked" on evidence it can actually read.
Change: one clause in the PROJECT_FACTS block (`.fabro/workflows/develop/prompts/` facts include or `reviewer.md`): "fs_hide binds planner/implementer; the reviewer node is deliberately unhidden."
Expected effect: removes a latent false-blocking path; reviewers stop journaling surprise about their own envelope.
New-seed justification: fabro-a512 corrects disproved fs_hide claims in `workflow.fabro` *comments* only; no open seed covers the PROJECT_FACTS prose accuracy for the reviewer envelope.

One smaller observation not worth a recommendation: the implementer's `nu -c` quoting failure on its negative test (journaled, 1 shell error) is the same pattern fabro-d950 files for the reviewer — worth folding into fabro-d950's scope as a one-line arm when it lands, rather than a new seed.
