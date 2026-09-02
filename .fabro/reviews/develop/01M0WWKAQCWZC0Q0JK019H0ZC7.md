# Improve review — run 01M0WWKAQCWZC0Q0JK019H0ZC7

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (6.0 min, revisor pass — reason and cost in run detail)
- generated: 2026-09-02 20:13+0000 by revisor `fabro_ask`

---

# Recommendations for the Develop workflow — run 01M0WWKAQCWZC0Q0JK019H0ZC7

Baseline (from run events, `run.completed` seq 259): 293s wall, 230s of it model inference, 18s tools, $0.271 total. Stage actives: planner 76.6s, implementer 149.5s, tester 1.0s, evidence 0.4s, reviewer 20.2s, closeout 0.5s. Zero retries, 6/6 green — so every recommendation below targets latency, cost, and latent risk, not failures.

## 1. Fix the PR postlude — it burns ~53s after the graph is done and fails twice every run
**What happened:** exit completed at 16:43:46 (seq 254); `run.completed` at 16:44:39. The 53s in between was PR creation whose **LLM body generation failed** ("PR content generation failed… `No object generated: Failed to parse response as JSON`", worker log 16:44:33) and fell back to a skeleton, followed by an **auto-merge attempt the repo forbids** ("Auto merge is not allowed for this repository", worker log 16:44:35) even though `pull_request.auto_merge=true` is set in the run spec. Net: 18% of wall time spent producing a fallback PR body and a guaranteed-failing GraphQL call.
**Change:** run settings `pull_request` block (run spec; `workflow.toml` doesn't override it): set `auto_merge=false` (or enable auto-merge in the GitHub repo), and either fix or disable the PR-body model call — the deterministic fallback is what ships anyway.
**Effect:** run reaches `completed` ~50s sooner; two warn-level failures disappear from every run.

## 2. Implementer: ban placeholder appends and require `timeout_ms` on compile/test shells
**What happened:** the implementer appended a knowingly-mangled placeholder test via `cat >>` heredoc (seq 138, its own next message: "That placeholder was sloppy"), tried to remove it with `head -n -28` and cut the wrong lines leaving a dangling comment (seq 144–158, two extra inspection rounds), then chained the real append + `gofmt; go vet && go test ./...` in one shell call **without `timeout_ms`** — it died at the default 10s timeout with zero output (seq 163–164, `is_error`, 10,345ms), forcing a blind re-run (seq 168). That churn window (16:41:33→16:42:21) is ~48s, 6 wasted tool calls, ~5 wasted LLM turns — a third of the implementer stage.
**Change:** `.fabro/workflows/develop/prompts/implementer.md`, "Your job this pass": add a hard rule — "write the final code block in one edit; never append placeholder code to fix later" and "any shell call that compiles or tests MUST pass `timeout_ms ≥ 60000` — the default 10s kills a cold `go vet` (observed: append+gofmt+vet+test timed out at 10.3s with no output, this run)."
**Effect:** implementer pass ~100s instead of 150s; eliminates the silent-timeout failure class.

## 3. Enforce the existing "no full gate" rule — the implementer ran the gate three times anyway
**What happened:** the prompt's item 4 says "compile or ONE focused test — nothing that formats, lints, or runs the full suite." The implementer ran `gofmt -l; go vet && go test .` twice plus `go build` + binary smoke runs (seq 162, 168, 181). The tester then re-ran the identical gate in 1.02s (cached). The forbidden chain is also exactly what timed out in rec 2.
**Change:** same file, promote item 4 from a numbered-list sentence to its own **"Hard rules"** block (like Artifact hygiene), stating the tester owns it and a cached gate re-run proves nothing.
**Effect:** fewer long shell chains (fewer timeouts), no triple-gating of one tree, cleaner role boundary.

## 4. Raise `preamble_budget_kb` 24 → 32 — the blob detour it was meant to kill is back
**What happened:** the graph bumped the budget 12→24 specifically so a ~14KB capture would render inline ("the blob detour is [the constraint]", `workflow.fabro` graph comment). This run's capture is 22.1KB (evidence `output_bytes`, seq 209) and rendered as "Output (22.9 KB; full value: …/blobs/3aedadc…)" in the reviewer prompt (seq 218) — 22.9KB + tester section + context keys (~28KB aggregate) exceeds 24KB, so the evidence was demoted again. The reviewer recovered via one `read_file` of the blob (seq 228, 102ms) and approved — but every review now walks the "unread blob ref" tightrope that produced the false rejection in run 01M0SS23MJ.
**Change:** `workflow.fabro` line 10: `preamble_budget_kb=32` (32,768 > ~28KB observed aggregate, ~5KB headroom).
**Effect:** reviewer reads the capture directly in the preamble; −1 tool round-trip per review; removes the preview-misread failure mode.

## 5. Make the implementer journal report tool failures — the timeout above shipped as `"none"`
**What happened:** the implementer's journal was `{"painpoints":[],"observations":["none"]}` (seq 187) despite a timed-out shell call, an `is_error` grep, and a signature change that forced edits at ~28 test call-sites — all literal "surprise/near-miss" material per its own prompt. The stage-journal hook is the only durable channel to the improve workflow; this signal was lost (only this analysis recovered it).
**Change:** `prompts/implementer.md`, Journal section: "a tool call that errored or timed out ALWAYS goes in `observations`, even if you recovered — name the call, the timeout, and the workaround."
**Effect:** the loop self-reports friction like rec 2's timeout, so prompt fixes get discovered by the journal pipeline instead of by archaeology.

## 6. Planner fast-path when the goal names the seed
**What happened:** the goal names `fabro-f74b`; `sd ready` returned exactly that one seed (seq 39). The planner still ran `sd list --format json` purely to double-check the count for its journal observation (its reasoning trace at seq 67 says so), after `sd show` + `sd update` + two globs + a full 10.4KB read of `main.go` — 7 tool calls and 75s inference for a single-candidate claim. The 30s ambiguity deliberation itself was valuable (it resolved both spec ambiguities the reviewer later verified); the waste is the redundant tracker call and the ready/list double-read.
**Change:** `prompts/planner.md`, step 1: "If the goal names a seed id, `sd show <id>` directly; run `sd ready` only when it doesn't, and never run `sd list` to re-confirm a count `sd ready` already printed."
**Effect:** −2 shell calls and one LLM turn (~5s + tokens) on named-seed runs, which is the common case for this workflow's goals.

## 7. Derive `files_touched` from the stage git diff, not from write-tool calls
**What happened:** the implementer stage metadata reports `files_touched: [README.md, main.go]` (seq 190) while the actual stage diff — and the evidence capture — shows `fib_test.go +159/-28`, the **largest** product change of the run. It's missing because `fib_test.go` was edited through shell (`perl`, `head`, `cat >>`), invisible to write-tool instrumentation. Any consumer keying on `files_touched` under-reports the implementer's real footprint.
**Change:** engine-side — compute `files_touched` from the stage checkpoint's git diff (the data already exists per checkpoint, seq 194).
**Effect:** accurate per-stage attribution; no silently missing files in stage metadata.

## 8. Scope skill discovery per role
**What happened:** all three agent stages loaded 41 irrelevant skills (~1.6–1.9k tokens each: `ask-matt`, `claude-handoff`, `scaffold-exercises`… from `/storage/.home/skills`, visible in every `context_window.breakdown` "skills" category). None was activated; the read-only reviewer needs none of them.
**Change:** per-node skill allowlist (or drop the global skills dir for `reviewer`/`planner`) in the agent/runner config.
**Effect:** ~5k tokens less boilerplate across the run and lower risk of an irrelevant `use_skill` misfire — the "bare slash-word" crash rule exists because of exactly this surface.

**Not inspected:** the internals of the PR-body generation failure (only the worker-log warn line and the fallback PR #3 are visible), and whether GitHub repo settings could enable auto-merge instead of disabling the flag — both are outside this run's event stream.
