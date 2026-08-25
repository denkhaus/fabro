# Improve review — run 01M0X2XC9AJQ96VVB70K6GN7QR

- workflow: hello
- branch integrated: denkhaus-lab
- status: succeeded (completed), 1.0 min, $0.038733
- generated: 2026-08-25 20:32+0200 by `fabro ask` with `scripts/prompts/improve.md`

---

# Improve-review — run `01M0X2XC9AJQ96VVB70K6GN7QR` (workflow `hello`)

**What actually happened** (from run events): `start → greet → exit`, no retries, no failures. Wall time 61.7s, cost $0.0387, 70,431 tokens (56,448 cache-read ≈ 80%). The `greet` agent stage took 51.9s wall — **48.7s of it inference (94%), 1.3s tool time**, across 7 sequential LLM turns / 6 tool calls. Setup was 838ms and sandbox 2.7s (the baked runner image worked as designed). The final diff: 4 lines appended to `README.md`, PR #5 auto-opened.

Recommendations, ordered by expected impact:

## 1. Prompting: disambiguate append-vs-replace in `greet` — kills ~24s of archaeology
**File:** `.fabro/workflows/hello/workflow.fabro:8`
**Evidence:** The prompt says only "Add a short haiku to README.md". The README already had a `## Haiku` section from run `01M0X072Y984SMZ3ENNP26HS44`, and the agent's reasoning (events seq 48) explicitly weighed "1. Replace the existing haiku / 2. Add another". Resolving that ambiguity consumed two shell calls (`git log -- README.md` seq 49, `git show 9c1154c` seq 55) and three LLM turns — roughly 18:29:15→18:29:40, ~24s of the 49s inference.
**Change:** `prompt="Append one new 5-7-5 haiku under the existing '## Haiku' heading in README.md (create heading/file only if missing). One haiku per run; do not modify earlier haikus. Do not build, test, or run any project tooling."`
**Effect:** Removes the entire archaeology segment (~35% of greet latency); reruns become deterministic instead of relying on the model to infer history.

## 2. Tool usage: drop the existence probe and read only the file tail
**Evidence:** Turn 1 (seq 38) was `ls README.md && echo EXISTS` after 8.0s of inference — pure waste, `read_file` answers existence itself. Turn 2 (seq 45) read the **entire 282-line / 11,709-byte README** including the giant flag-reference table; conversation tokens jumped 456 → 3,796 for a task whose anchor was the last 10 lines.
**Change:** With rec 1's wording the probe is unnecessary; additionally note in the prompt "the haiku section is at the end of the file" so the agent reads with `offset`/`limit` or greps for `## Haiku` instead of whole-file reads.
**Effect:** One fewer LLM turn (~8s) and ~3.3K fewer conversation tokens per run.

## 3. Graph design + error handling: add a deterministic `verify` script node
**Evidence:** The only correctness check in the whole run was the agent's own `git diff` (seq 66). `greet` has `max_attempts=1`, both edges are unconditional, and there is no failure path anywhere — a silent no-op (agent replies nicely, edits nothing) would still be reported `succeeded`. The stated goal is "demonstrate a basic Fabro workflow", yet the graph demonstrates only a single agent node.
**Change:** In `workflow.fabro`, insert `greet -> verify -> exit` with a script node, e.g. `git diff --numstat -- README.md` asserting exactly `4	0` (or `grep -c` the heading). It runs as workflow infrastructure, not "project tooling", so it doesn't conflict with the prompt's no-tooling guardrail — which, credit where due, the agent honored correctly (seq 37/65 reasoning on precedence over AGENTS.md's `sd`/`ml` rituals).
**Effect:** Silent failures become loud ones; the demo actually shows a second handler type and a gate.

## 4. Latency: fewer turns, lower reasoning effort for a trivial node
**Evidence:** 7 sequential turns with 4–10s time-to-first-output each; **every turn issued exactly one tool call** (no batching), and 1,037 reasoning tokens were spent on a 4-line edit. `controls.reasoning_effort` is `null` in the run spec.
**Change:** Set a low `reasoning_effort` for this node's model controls, and add "verify with a single `git diff`, then finish" to the prompt so the verify-and-summarize turns collapse into one.
**Effect:** Greet wall time from ~52s toward ~25–30s; this is the run's dominant cost, since tool time was only 1.3s.

## 5. UX: PR creation is 20% of a "hello" run
**Evidence:** `exit` completed 18:30:03.4; `pull_request.created` landed 18:30:16.5 — ~12s inside the 61.7s run — and every rerun of this demo opens a new auto-merge PR (PR #5) for a 4-line haiku. Metadata snapshots add another 8.1s (2.9+3.4+1.8s, ~13% of wall; only partially tunable via checkpoint settings).
**Change:** Disable `pull_request` for the `hello` workflow (or gate PRs on diff size); keep run/meta branch pushes for auditability.
**Effect:** ~12s faster perceived completion and no per-demo PR noise.

## 6. Minor platform bug worth reporting
**Evidence:** Checkpoint seq 79 recorded `node_outcomes.greet.timing.wall_time_ms = 0` while `stage.completed` (seq 75) correctly recorded 51,894ms.
**Effect if fixed:** Per-node timings in checkpoint/audit views stop under-reporting agent stages.

**What to keep as-is:** the no-tooling guardrail wording (worked exactly as intended), the baked runner image (setup collapsed to 838ms), and the caching setup (80% cache-read tokens).

*Sources: run events seq 1–109 (stage transcript, timings, billing, checkpoint/PR lifecycle) and workspace file `.fabro/workflows/hello/workflow.fabro`.*
