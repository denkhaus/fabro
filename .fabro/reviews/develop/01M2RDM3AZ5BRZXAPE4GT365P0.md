# Improve review — run 01M2RDM3AZ5BRZXAPE4GT365P0

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (10.4 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-17 19:45+0000 by revisor `fabro_ask`

---

All evidence gathered. Here are the recommendations, ordered by expected impact, each grounded in this run (seed fabro-7daf, PR #230; total 10m25s wall / $1.004; implementer alone was 481s wall and $0.678 — 78% of wall, 67% of cost — per the run conclusion's stage table).

---

## 1. Ship the nushell-scripts skill — the implementer's biggest cost this run was re-deriving nu 0.115 semantics
**Evidence (from run events + journal):** the implementer made 43 shell calls with **8 errors**, and its journal painpoint states "nu 0.115 module composition cost three probe cycles: hyphenated module filenames break `use file.nu <alias>`, `use`-imported commands are invisible inside sibling top-level defs, and `source` is parse-order dependent." Result: 467s inference against 13s tool time for a 128-line script change (recorded as mx-ec83d3).
**Change:** create `.fabro/skills/nushell-scripts/SKILL.md` (mx-ec83d3's three gotchas are ready-made content) and add one line to `.fabro/workflows/develop/prompts/implementer.md` step 2: load the skill when the seed touches `.nu` files.
**Expected effect:** 1–2 min and $0.03–0.08 saved per script seed, fewer errored shell calls.
**Seed:** **fabro-d19e** (open) covers exactly this; this run is fresh confirmation it hasn't landed yet.

## 2. Make evidence.nu render binary-classified diffs as text
**Evidence:** the reviewer's journal painpoint (from run events): the loop-work diff showed `.fabro/scripts/planner-preflight-anchor-fixtures.nu` as "Binary files differ" (the fixture deliberately embeds a NUL byte for the binary-degradation case), so the reviewer had to shell-read the 93-line file and re-run the 17-check battery itself — 7 of its 9 tool calls were this workaround class.
**Change:** in `.fabro/workflows/develop/scripts/evidence.nu`, when a changed file's diff renders as "Binary files differ", fall back to `git diff --text` (or an escaped/base64 rendering) under the same size bounds.
**Expected effect:** reviewers stop needing a per-review shell workaround for binary-classified deliverables; removes the "UNSEEN item" ambiguity that can otherwise escalate to Verification blocked.
**Seed:** none exists — greps of `.seeds/issues.jsonl` for "Binary files differ", "git diff --text", and NUL/binary-capture return nothing (fabro-020b caps diff *size*, fabro-a494 covers off-tree branches). **New-seed justification: no open seed covers binary-classified diff rendering in the evidence capture.**

## 3. Raise the reviewer's inline cap — this run's 17.0 KB capture was demoted for being 1 KB over
**Evidence:** the reviewer's stage prompt shows `Output (17.0 KB; full value: /tmp/sandbox-driver/runtime/blobs/3c441eff…)` — the evidence capture was blob-ref'd by `preamble_inline_max_kb=16`, forcing the blob-reading detour the graph comment (fabro-1e9f, "zero blob detours") explicitly says it wants to avoid.
**Change:** in `.fabro/workflows/develop/workflow.fabro`, reviewer node: `preamble_inline_max_kb=16` → 32 (the seed's 2026-09-16 update says aim ≥40).
**Expected effect:** captures of this size arrive inline; one fewer tool round trip per review and the unread-blob rejection class disappears.
**Seed:** **fabro-cf3e** (open) — this run adds a fresh 17 KB data point to it.

## 4. Backfill pull_request.state so the planner's in-flight guard stops guessing on null
**Evidence:** the planner's journal observation (from run events): `fabro_runs_list` showed `pull_request.state: null` for PRs 227/225/223 (succeeded runs with real PRs) — treated as "not in-flight" under the only-"open"-counts rule, i.e., those seeds were double-pickable if any PR was actually open.
**Change:** engine-side, populate `pull_request.state` for terminal-but-unmerged runs in the `fabro_runs_list` projection (store/projection code).
**Expected effect:** the planner's in-flight guard stops running blind exactly when a PR sits unmerged; prevents the fabro-22e4/fabro-91ff duplicate-claim class.
**Seed:** **fabro-4bb7** (open) — this run is a third live occurrence of its exact symptom.

## 5. Enforce the reviewer's read-only posture as a capability, not prose
**Evidence:** the reviewer node — "read-only BY CAPABILITY AND POLICY" — had the entire mutation-capable registry available (event seq 391) and invoked `fabro_run_interact` (seq 440; `action: "get"`, benign this time, but the approve/deny/cancel path was one argument away). Meanwhile the planner needed an explicit node-level `fabro_tools` opt-in, so the graph's tool posture is inconsistent across nodes.
**Change:** add `tools="read_file,grep,glob,shell"` and an empty/minimal `fabro_tools` to the reviewer node in `.fabro/workflows/develop/workflow.fabro` (and consider the same for the implementer minus `spawn_agent`).
**Expected effect:** review's read-only contract becomes mechanically enforced; capability-delta risk (ADR-0019 family) shrinks to what the diff adds, not what the toolset allows.
**Seed:** engine capability landed via **fabro-47b5** (closed), which lists "reviewer read-only" as a lab use case — but no open seed tracks adopting it on the develop nodes. **New-seed justification: no open seed covers the develop-workflow adoption of the per-node tool allow-list.**

---

**Not recommended despite being visible:** the 5.3s gate run, 0.3s evidence, and 0.4s closeout were all cheap and green first-try — no graph or gate change is warranted there on this run's evidence. The planner (50s, $0.086, 4 tool calls) also performed within its measured envelope.

Sources: stage timings/usage from the run conclusion and checkpoints (run events), implementer/reviewer journal entries (run events + committed `.fabro/journal/01M2RDM3AZ5BRZXAPE4GT365P0.jsonl`), the reviewer's blob-ref marker (stage prompt, run events), the `fabro_run_interact` call (event seq 440–441), and seed lookups from workspace file `.seeds/issues.jsonl`.
