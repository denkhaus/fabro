# Improve review — run 01M2QBA0BYJK4B241YG5RPCZHG

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (17.7 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-17 10:10+0000 by revisor `fabro_ask`

---

## Recommendations for this Develop workflow — grounded in run 01M2QBA0BYJK4B241YG5RPCZHG

**Run baseline (from run events/conclusion):** 17m47s wall, $0.723 total. Seed fabro-9f97 claimed → implemented (+73/−2 in `lib/components/fabro-workflow/src/pipeline/publish.rs`) → gate green first pass (28.6s) → approved first pass → closed, PR #215. Cost split: implementer $0.454 (63%, 841s wall / 522s tool / 39 tool calls), planner $0.219 (30%, 130s), reviewer $0.050. The loop shape is healthy — every recommendation below trims friction inside a working loop.

---

**1. Stop the implementer from re-running what `just verify implementer` and the tester re-run anyway. — seed fabro-2b1b (+ fabro-7f58)**
What happened: the planner's brief for fabro-9f97 prescribed hand `cargo fmt` + `clippy` + full `cargo nextest run -p fabro-workflow` (1,529 tests), then the implementer also ran `just verify implementer`, which re-runs the pinned fmt+clippy per touched crate — and the tester re-ran the gate 23 minutes into the run. That's ~2 redundant compile/test passes inside the implementer's 522s tool time (63% of run cost) to validate 3 new unit tests.
Change: `.fabro/workflows/develop/prompts/planner.md` step 6/7 and `implementer.md` step 4 — briefs prescribe `just verify implementer` plus genuine supplements only; for purely *added* tests run them by name filter (fabro-7f58).
Expected effect: one fewer compile pass + drop the 1,529-test sweep to ~3 named tests per test-adding seed — roughly 1–2 min wall and one LLM turn saved per run, gate coverage unchanged.

**2. Normalize stale mtimes before the implementer's own builds, not just at gate start. — seed fabro-56db (scope extension this run proves out)**
What happened: the implementer's journal (mx-1e5205) records cargo nextest serving a **stale binary** after `edit_file` changes through the `/workspace/fabro` symlink — a freshly-fixed test kept failing with the pre-fix assertion until a manual `touch` forced rebuild. That silent staleness is exactly the failure mode behind most of the 522s tool time.
Change: mirror fabro-56db's one-liner (`find lib -name '*.rs' ! -newermt 2000-01-01 -exec touch {} +`) into `implementer.md` step 4 (pre-verify), not only `scripts/qualitygate.nu`.
Expected effect: eliminates the stale-binary misdiagnosis class at the stage where it actually bit this run; zero LLM cost.

**3. Make the planner's stale-path hunt deterministic. — seeds fabro-4c81 + fabro-55a7**
What happened: seed fabro-9f97 named `.fabro/project.toml [run.pull_request]` — a section that no longer exists. The planner then burned ~7 sequential probe rounds (seq 51–100: grep project.toml → exit 1 → `ls .fabro` → repo-wide `grep auto_merge` → grep pull_request.rs → grep finalize.rs → sed) to find the real target, `publish.rs` ~line 153. That hunt drove most of the planner's 130s/$0.219 (30% of run cost) — a sub-second greppable fact.
Change: extend `.fabro/scripts/dup-run-check.nu` (or sibling `claim-check.nu`) per fabro-4c81 so every repo path named in a seed body resolves before the claim is legal; batch remaining recon into one shell call per fabro-55a7.
Expected effect: planner drops from ~7 probes to 2–3 calls (~60s, ~$0.10 saved per pass); each stale-path seed gets corrected once in-tracker instead of re-derived by every later planner.

**4. Fix the evidence tail-truncation the reviewer worked around. — seed fabro-meta-c9f2 (this is occurrence 5)**
What happened: the evidence capture (9,890 bytes, well under the 16KB inline cap) still rendered to the reviewer with **"(81 lines omitted)"** — the reviewer's own journal says it "Verified the gate directly in `publish.rs` (line 154) rather than trusting the truncated evidence preview." The approval was correct, but the reviewer judged from tools, not from the capture the evidence node exists to deliver; on a weaker reviewer this class has already caused rejection cycles (see the seed's four prior occurrences).
Change: rearchitect `.fabro/workflows/develop/scripts/evidence.nu` per fabro-meta-c9f2 — budget to the *rendered* tail_lines window (diff-first, header compact), or render full output on the reviewer node.
Expected effect: one-cycle approvals on visible evidence; reviewer stops paying tool round-trips to re-derive what the capture already contained.

**5. Treat the preflight verdict table as the candidate source; stop the `sd ready` firehose. — seed fabro-c3b4**
What happened: the `preflight` node (1.0s, deterministic) already computed a clean 5-candidate table (`output.preflight`), yet `planner.md` step 1 mandates `sd ready --assignee fabro --limit 200` as the first tracker call — which poured **28,577 bytes / 200 seed lines** into the planner conversation (event seq 38) just to re-learn the top of the queue.
Change: `.fabro/workflows/develop/prompts/planner.md` step 1 — when `output.preflight` is `mode=checked` and non-degraded, pick from its table (falling back to `sd ready` only for degraded/insufficient tables); this is the strongest form of fabro-c3b4's top-N view.
Expected effect: ~28KB less planner context per pass, fewer distracted probes, shorter claim-to-dispatch window (which also narrows the duplicate-claim race fabro-a01f).

**6. Silence the false ERROR/WARN noise in the worker log. — seeds fabro-a701 + fabro-8275**
What happened: the persisted worker log for this clean, first-pass-green run contains **6 ERROR lines** — all `File "/workspace/fabro/.codex/instructions.md" was not found` at agent session init (2× per stage session) — plus a WARN for the by-design absent `output.gate_known_bug_hits` key on a first (green) implementer visit. Zero of these indicate anything wrong.
Change: per fabro-a701, log absent *optional* memory files at info in agent session init; per fabro-8275, downgrade the allow-key-absence warn to info.
Expected effect: a green run's warn+ log goes to ~0 lines (currently 33), so real signal — like the stale-mtime or truncation issues — isn't buried.

**7. File the reviewer's residual finding before the seed closed. — new-seed justification: no existing seed covers it (checked tracker for rsplit/diff-path/quoting seeds; fabro-7aac covers implementation-summary deferred actions only, not reviewer-journal findings on a closing seed).**
What happened: the reviewer's journal flagged a real defect in code shipped *this run* — `diff_header_b_path`'s `rsplit_once(" b/")` can mis-split paths containing a literal `" b/"` segment — as "noted but not blocking." The seed then closed at closeout; the observation now lives only in `.fabro/journal/01M2QBA0BYJK4B241YG5RPCZHG.jsonl`, and the planner's journal-observation rule has no next brief to fold it into.
Change: (a) file the follow-up seed (quote-aware `b/`-path parsing in `lib/components/fabro-workflow/src/pipeline/publish.rs`; git quotes such paths in real diffs, so the fail-closed arm silently reverts to PR-creation on space-bearing paths); (b) mechanism-wise, extend the closeout re-file pattern of fabro-7aac to also sweep reviewer-journal "noted but not blocking" findings into seeds at `closeout.nu` time.
Expected effect: no verified residual defect ever dies with its closed seed; the journal stops being a graveyard of orphaned findings.

**8. Carry tool-result errors through the provider protocol. — seed fabro-b09c**
What happened: the planner's probe `grep … .fabro/project.toml` exited 1 (legitimate no-match — the stale path) and the tool layer set the error flag, producing **12 duplicated `unsupported_control` warnings** in this run's log (one pair per errored call). The model only sees failure in stdout text, which is what sent it into the 7-probe hunt in recommendation 3 and historically causes blind retries.
Change: engine agent-session/provider layer per fabro-b09c — propagate the tool error flag where the provider supports it, emit an explicit failure marker where it cannot.
Expected effect: deterministic error signaling; the warning spam disappears and errored calls stop being misread as sandbox unreliability.

**9. Add a long-stage heartbeat. — seed fabro-b769**
What happened: the implementer ran **14 of the run's 17.7 minutes** with no external signal — `workflow.toml` notifies only on `run.completed`/`run.failed` — and at $0.454 it was 63% of run cost with no operator hook to intervene.
Change: `.fabro/workflows/develop/workflow.toml` notifications block per fabro-b769 — add a "stage exceeds N minutes" heartbeat alongside terminal events.
Expected effect: mid-run visibility into single-stage cost concentration while cancellation is still possible.

---

**What already worked (no change):** the deterministic `preflight` node (1s), `closeout` via stdin (0.4s, closed exactly fabro-9f97), gate warm-cache discipline (tester green in 28.6s cold-ish vs the ~15m worst case), the stale-basis `sd update --description` correction recorded *before* the claim (the planner did apply it this run — fabro-4c81's prompt mandate held here), and the reviewer's token economy (3 tool calls, $0.050).
