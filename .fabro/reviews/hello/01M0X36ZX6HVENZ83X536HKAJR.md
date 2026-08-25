# Improve review — run 01M0X36ZX6HVENZ83X536HKAJR

- workflow: hello
- branch integrated: denkhaus-lab
- status: succeeded (completed), 0.7 min, $0.028215
- generated: 2026-08-25 20:37+0200 by `fabro ask` with `scripts/prompts/improve.md`

---

# Improve-review — run `01M0X36ZX6HVENZ83X536HKAJR` (workflow `hello`)

**What actually happened** (from run events): clean success, `start → greet → exit`, zero retries. Created 18:34:08.7 → completed 18:35:15.1 (66.4s; 61.7s started→completed). The `greet` stage: 32.0s wall, **28.8s inference (90%), 0.76s tools**, 5 LLM turns, 6 tool calls. Cost $0.0282, 43,393 tokens (31,488 cache-read ≈ 73%). Setup excellent: sandbox ready 2.8s, `mise install` a 149ms no-op (baked image worked as designed), bootstrap 722ms. Diff: 4 lines appended to `README.md`, PR #6 auto-opened.

**Meta-finding first:** this is the **third** improve-review of `hello` (prior: `.fabro/reviews/hello/01M0X072Y984SMZ3ENNP26HS44.md`, `01M0X2XC9AJQ96VVB70K6GN7QR.md`), and this run re-paid the same costs both flagged: existence probe (seq 37–41), full-file read (seq 46), no verify node, ~16s PR tail, 35 unused skills. The fixes below are one-file, one-line changes — landing them before the next rerun is the highest-impact action available.

## 1. Prompting: the "create if missing" conditional is dead weight — third run, still paying for it
**File:** `.fabro/workflows/hello/workflow.fabro:8`
**Evidence:** README has existed in all three runs; the conditional still drove turn 1: a `glob README.md` **plus a redundant `ls /workspace/fabro`** in the same turn (seq 37–41), then a full 285-line / 11,811-byte `read_file` (seq 45–46) to locate the haiku section — conversation tokens jumped 53 → 4,659.
**Change:** `prompt="Append one new 5-7-5 haiku under the existing '## Haiku' heading at the end of README.md (create heading/file only if missing); read only the file tail. One haiku per run; do not modify earlier haikus. Do not build, test, or run any project tooling."`
**Effect:** 5 turns → 3; removes the probe and the whole-file read (~10s, ~30% of stage tokens).

## 2. Prompting: resolve the AGENTS.md ritual conflict the agent deliberated over twice
**File:** same prompt line (or a carve-out in `AGENTS.md`)
**Evidence:** reasoning traces seq 49 and seq 61 both weigh `sd prime`/`ml prime`/`ml record` (AGENTS.md) against the prompt's no-tooling clause. Turn 3 alone: **10.0s to first output, 531 reasoning tokens** — syllable-counting plus this precedence debate. The agent resolved it correctly both times, but a future run may not.
**Change:** add one sentence: "The no-tooling rule includes AGENTS.md's sd/ml session rituals — skip them for this task."
**Effect:** shorter reasoning turns; eliminates a nondeterministic instruction conflict.

## 3. Graph design + error handling: add a deterministic `verify` node (recurring, unaddressed twice)
**File:** `workflow.fabro` (graph)
**Evidence:** the *only* correctness check in the run was the agent's own `git diff` (seq 55–57). All edges unconditional, `max_attempts=1` (seq 26) — a silent no-op or double-append would still report `succeeded`. The goal is "demonstrate a basic Fabro workflow," yet only the agent handler is demonstrated.
**Change:** `greet -> verify -> exit` with a script node, e.g. `script="git diff --numstat -- README.md | grep -q '^4\s' ` (asserts exactly one appended haiku). Runs as workflow infrastructure, not "project tooling."
**Effect:** silent failures become loud; the demo shows a second handler type and a gate for ~0.2s.

## 4. UX/latency: 25s of post-work tail is 40% of the run (recurring)
**Evidence:** greet completed 18:34:50.2 → `run.completed` 18:35:15.1 (24.9s): checkpoint snapshot 3.3s, finalize snapshot 1.9s, **PR creation ~16.3s** (push 18:34:56.95 → `pull_request.created` 18:35:13.21), plus a duplicate run-branch push (seq 70 and 73, 1.6s apart). PR #6 is the third auto-merge PR for a 4-line haiku.
**Change:** disable PR for this workflow only (`.fabro/workflows/hello/workflow.toml`: `[run.pull_request] enabled = false`); platform-side, push once after `exit`.
**Effect:** ~16–20s off perceived completion; no per-demo PR noise.

## 5. Tool usage/context: 35 skills, zero activated — ~16% of live context (recurring)
**Evidence:** `agent.skills.discovered` seq 30 (35 skills); skills category 1,688–1,716 tokens of a ~10.6K-token live context at every turn (seq 61 breakdown). This graph can never use a skill.
**Change:** scope/disable skill discovery for `hello` in its run settings.
**Effect:** ~1.7K tokens off every turn's context at zero capability loss.

## 6. Platform bug, second occurrence: inconsistent timing fields
**Evidence:** checkpoint seq 68 records `node_outcomes.greet.timing.wall_time_ms = 0` while `stage.completed` seq 64 correctly records 32,022ms (same bug flagged after run `…GN7QR`). Also `run.completed` reports `wall_time_ms: 41463` vs 61.7s observed started→completed — appears to exclude PR/finalize phases.
**Effect if fixed:** per-node timings in checkpoint/audit views stop under-reporting; run wall time reflects what the user actually waited.

**Keep as-is:** the baked runner image (setup collapsed to <1s), the no-tooling guardrail wording (honored correctly, including over AGENTS.md), the caching setup (73% cache-read), and the closing summary — unlike run `…SMZ3E`, it no longer tells the user to commit when Fabro already did.

*Sources: run events seq 1–100 (stage transcript, timings, billing, PR lifecycle), workspace files `.fabro/workflows/hello/workflow.fabro`, `.fabro/workflows/hello/workflow.toml`, and the two prior reviews under `.fabro/reviews/hello/`. Could not inspect: the PR-body generation internals (no per-call events inside the 16.3s gap) and whether `workflow.toml` supports the `[run.pull_request]` override (rec 4 names the intended key; verify against the fabro config schema).*
