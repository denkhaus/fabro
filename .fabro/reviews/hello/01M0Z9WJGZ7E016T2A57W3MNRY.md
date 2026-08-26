# Improve review — run 01M0Z9WJGZ7E016T2A57W3MNRY

- workflow: hello
- branch integrated: denkhaus-lab
- status: succeeded (completed), 0.6 min, $0.027452
- generated: 2026-08-26 17:13+0200 by `fabro ask` with `scripts/prompts/improve.md`

---

# Improve-review: run 01M0Z9WJGZ7E016T2A57W3MNRY (Hello)

**What actually happened** (from run events): a 3-node linear graph (`start → greet → exit` in `.fabro/workflows/hello/workflow.fabro`) appended a 4-line haiku to `README.md`. The run succeeded, 0 retries, $0.0275 total, 10,737 input / 312 output / 640 reasoning tokens, 31,680 cache-read. The telling ratio: **28.9s inference vs 0.48s tool time** — the run is entirely LLM-bound, and the recommendations follow that.

## 1. Cut `greet` from 5 LLM round-trips to 3 — the single biggest lever
**File/node:** `.fabro/workflows/hello/workflow.fabro`, node `greet`.

The agent made 5 inference calls: (1) `glob README.md` exists-check (~6.2s), (2) full-file `read_file` of all 289 lines / 11.9KB to find the Haiku section (~5.6s), (3) narration + `edit_file` (~7.9s), (4) verify `read_file` at offset 277 (~3.3s), (5) final summary (~3.7s). Give the node the facts it had to discover: *"README.md exists at the repo root; its `## Haiku` section is the last section. Read only the tail (offset ~270), append one 5-7-5 haiku, then stop."* That eliminates the glob turn and shrinks the read.
**Expected effect:** ~10–14s less wall time and roughly 30–40% off the stage cost; no behavior change (the diff is identical).

## 2. Set `reasoning_effort` low (or a lighter model) for edit-only nodes
**File:** `.fabro/workflows/hello/workflow.toml` / run spec (`run.model.controls.reasoning_effort` is currently `null`).

640 reasoning tokens were burned on a 4-line append. Worse, the first turn spent 186 reasoning tokens (visible in the seq-36 reasoning trace) deciding whether AGENTS.md's "run `sd prime` at session start" applies — precedence lawyering the model had to do because the node prompt's "Do not build, test, or run any project tooling" clause exists *solely* to override it.
**Expected effect:** trims 1–3s and a few hundred tokens per turn; more importantly makes trivial-node latency predictable.

## 3. Fix the AGENTS.md conflict at the source, not per-node
**File:** `AGENTS.md` (workspace root, loaded as 5,307 bytes of memory every session).

Keep the tooling prohibition in the graph only until AGENTS.md itself says *"edit-only runs skip `sd`/`ml` priming."* Then the defensive clause in the `greet` prompt can shrink to nothing and every future edit-only node inherits the rule for free.
**Expected effect:** shorter prompts, zero reasoning spent on precedence, and the rule is stated once instead of re-fought per node.

## 4. Replace the LLM self-verify turn with a script gate node
**File/node:** add a `verify` node after `greet` in `workflow.fabro`.

Turn 4 (~3.3s + part of turn 5) existed only to re-read 17 lines and confirm the model's own edit. A script gate (`rg -q '^merged before coffee$' README.md` or "diff adds exactly 4 lines and file ends with newline") costs milliseconds and — unlike the self-check — produces a *hard* failure signal. This also serves the run's stated goal ("demonstrate a basic Fabro workflow"): a linear graph with no gate demonstrates checkpointing but not Fabro's gating, which is the more interesting half.
**Expected effect:** −4s and one LLM call per run; real verification instead of trusted self-report.

## 5. Error handling: the graph has none, cheaply add some
**Files:** `workflow.toml` / run spec.

`greet` ran with `max_attempts=1` and `model.fallbacks={}`. Nothing failed here, but a single provider 429/timeout would have killed the run after the sandbox, clone, and setup were already paid for. Set `max_attempts=2` and one fallback model for agent nodes.
**Expected effect:** transient provider errors become a retry instead of a failed run; zero happy-path cost.

## 6. Post-work bookkeeping is ~27% of the run (engine-side)
From `greet` completion (15:10:00.76) to `run.completed` (15:10:15.28) — **14.5s** after all work was done: meta snapshot 2.0s, commit+push, a **second push of the same branch with unchanged head sha `8f1cb3b`** (seq 66 and seq 69, ~1.2s apart), PR creation 6.1s, finalize snapshot 1.8s. Two concrete changes in the engine/`scripts/run_workflow.nu` poll path: dedupe the redundant second push, and overlap PR creation with the finalize snapshot instead of running them serially.
**Expected effect:** −3–6s on *every* run, all of it pure overhead.

## 7. Slim the per-node context preamble
Final context was 10,597 tokens, of which **6,020 (57%)** was preamble: system 1,253 + tools 1,682 + skills 1,701 + memory 1,384. Zero of the ~40 discovered skills (`code-review`, `tdd`, `wayfinder`, …) were relevant or activated; only 3 of 14 tools were used. Scope skills per workflow (e.g., an empty/skills-less config for `hello` in `workflow.toml`) rather than inheriting all of `.fabro/skills/`.
**Expected effect:** faster first token on turn 1 and lower per-turn input; mostly latency, since cache-reads already absorb much of the cost (31,680 tokens cached — that part is working well).

## Not worth changing
- `mise install` was a 318ms no-op — the baked runner image (`image.dockerfile` header documents the 24.5s→no-op win) is doing its job. `just bootstrap` (1.2s) is arguably skippable for edit-only tasks, but it's below the noise floor.
- The prompt's "create the file if missing" conditional is dead in this repo and caused the glob turn, but it's harmless demo-defensiveness; fixing it is subsumed by recommendation 1.
