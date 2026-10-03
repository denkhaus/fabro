# Improve review — run 01M2X8458MDWMRVBRVEMDX9W4J

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (5.1 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-19 16:42+0000 by revisor `fabro_ask`

---

All evidence is in. Here are the recommendations, ordered by expected impact, each grounded in this run's events (`01M2X8458MDWMRVBRVEMDX9W4J`, seed `fabro-a1ef`, 5m03s wall, $0.365 LLM cost).

---

## 1. Fail prompt-lint in the implementer's verify lane, not at the gate — kills the red-gate bounce class
**What happened (run events):** tester@1 went RED (14.2s) on prompt-lint rule 6: `error: .../implementer.md: run id literal '01M2R8FHE6NQ14SGKXJ015JWPZ' — provenance belongs in the seed Basis`. Cause: the planner's brief transcribed the seed body's `(basis: run 01M2R8FHE6NQ14SGKXJ015JWPZ…)` parenthetical verbatim, and implementer@1 followed the brief literally. Recovery cost: gatebounce 0.2s + implementer@2 63.8s / $0.075 ≈ **78s wall (~27% of the run) for a one-line Markdown seed**. The reviewer confirmed the class: "Seed spec self-contradiction… planner briefs for prompt-editing seeds should phrase provenance semantically" (journal).
**Change:** add a deterministic arm to `scripts/verify.nu` (the `just verify implementer` dispatcher): when the diff touches any `.fabro/workflows/**/prompts/*.md`, run `.fabro/scripts/prompt-lint.nu` on exactly those files — the same lint `scripts/qualitygate.nu` runs — so the violation fails the implementer's own 28s pass instead of a full tester cycle. Pair with the hygiene clause fabro-41de already demands ("enforced at the point where prompts get edited"); per the fabro-9ec3 standing policy the mechanical arm is the compliant carrier, not prose.
**Seed:** **fabro-41de** (open — its extension demand (c) owns this exact class; the verify.nu arm is the script-side implementation of it).
**Expected effect:** prompt-editing seeds with quoted provenance stop costing a bounce cycle (~78s + $0.075 each, observed once in this run; the tracker's backlog is dominated by prompt-revision seeds).

## 2. Planner lap economy: stop feeding the planner firehoses
**What happened (run events, planner@1 = 99.3s wall, 83.2s inference, $0.218 — 60% of run cost):** the lap made 10 tool calls, 2 errored, and ingested three oversized outputs: `sd ready --limit 200` returned **28.6KB / 200 rows, stdout-truncated** (seq 46-47); `fabro_runs_list` returned 88 runs with `pull_request.state: null` on recent terminal runs (seq 49), degrading the in-flight check the prompt says reads "exactly open"; and a `git fetch origin/main` probe **failed** (`fatal: ambiguous argument 'origin/main'` — the fork clone has no `origin/main`; seq 61) plus a recovery `git branch -r` call that also errored (seq 68).
**Change:** implement **fabro-c3b4** (top-N `sd ready` view, N≈10, instead of the 200-row firehose) and **fabro-55a7** (batch `sd ready` + `sd show` + base-branch grep into one chained shell call); same-lap family: **fabro-4bb7** (backfill `pull_request.state` for terminal-but-unmerged runs so the in-flight check stops degrading) and **fabro-6b58** (bound the `fabro_runs_list` window).
**Expected effect:** fewer turns and far less input tonnage on claim-only laps — this run's planner burned 61.8k input tokens to emit a 1.2k-token brief; 30-50% planner wall/cut is the realistic band those seeds were filed from.

## 3. Memoize external-wait skips into the seed body (fabro-af22 / upstream PR #784)
**What happened (run events):** the planner adjudicated top candidate fabro-af22 (High): `sd show` (seq 55) → failed `origin/main` fetch (seq 61) → failed branch probe (seq 68) → `web_fetch` of the #784 GitHub page, **13.4KB of page chrome to learn one word: "Open"** (seq 74) — then journaled *"next planner should re-check #784 state each lap."* That is a standing per-run tax until upstream merges #784, and the git-based check is structurally impossible in this clone (no upstream remote).
**Change:** when the planner skips a candidate on an external-wait condition, persist the adjudication via `sd update <id> --description` (append-only note: "skip note: waiting upstream fabro-sh#784, verified open 2026-09-19, re-check only after upstream merge") so every later planner reads it from the `sd ready`/`sd show` it already does — zero extra calls, no re-derivation.
**Seed:** new-seed justification: no open seed covers skip memoization (fabro-af22 itself tracks the fix, not the recurring skip cost; fabro-4bb7/9372 cover the runs-list projection, not upstream-PR waits).
**Expected effect:** removes ~4 tool calls, 2 error recoveries, and a 13KB web fetch from every develop run while #784 stays open — recurring, unbounded horizon.

## 4. Markdown-only seeds: finish the gate tier split
**What happened (run events):** both gate runs printed `no crates touched` yet still ran `lint-nu` (34 scripts), prompt-lint (40 files), and workspace `cargo fmt --check --all` — 14.2s (red) + 22.2s (green) = **36.4s tester time (~12% of run wall) validating a 1-line Markdown diff**.
**Change:** implement **fabro-574d** in `scripts/qualitygate.nu`: derive the per-seed touched set and skip the entire Rust tier (including `fmt --check --all`) when no `lib/**` / `*.rs` / `Cargo.toml` changed.
**Expected effect:** ~10-20s off every prompt/config-seed run; compounding, since this line's backlog is overwhelmingly loop-asset revisions.

## 5. Loop-asset-only run PRs shouldn't run the project CI gate
**What happened (run events):** the run's final diff is 4 files, all loop assets (`.fabro/journal/…`, `.fabro/workflows/develop/prompts/implementer.md`, `.mulch/expertise/…`, `.seeds/issues.jsonl`) — yet PR #285 goes through the full project gate on GitHub, priced ~15 min worst-case for a diff that cannot affect Rust.
**Change:** implement **fabro-9495**: when the run diff intersects only loop-asset paths, mark the PR publish-eligible with a gate-skipping label (or merge without checks); code-carrying PRs unaffected.
**Expected effect:** ~15 min wall and CI compute removed per bookkeeping/prompt-seed PR — the user-visible half of the loop's turnaround.

## 6. Tighten gatebounce matching to error signatures
**What happened (run events):** gatebounce returned 3 hits; only fabro-41de was causal (implementer@2 credited it: "reading it first avoided re-deriving root cause"). fabro-f18a and fabro-9495 were keyword noise filling 2 of the 3 `maxItems` slots the bounce implementer must read.
**Change:** implement **fabro-c841** (open, verified in the tracker): match candidate seeds against the gate's `error:` signature lines, not the whole tail.
**Expected effect:** bounce passes start at the true root cause with ~2.4KB less noise per red gate; small but improves every future bounce.

---

**What worked and needs no change:** the gatebounce→implementer loop itself, evidence pipe (8.8KB capture rendered fully inline — reviewer needed zero tool calls, 18.2s, $0.028), and the deterministic nodes (tracker_guard 0.3s, claim_check 0.1s, closeout 0.4s) — all behaved as designed; recommendations 1 and 6 harden the paths around them rather than replace them.
