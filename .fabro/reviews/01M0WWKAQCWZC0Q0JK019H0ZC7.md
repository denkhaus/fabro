# Improve review — run 01M0WWKAQCWZC0Q0JK019H0ZC7

- workflow: develop
- branch integrated: denkhaus-lab
- status: succeeded (completed), 4.9 min, $0.271162
- generated: 2026-08-25 19:41+0200 by `fabro ask` with `scripts/prompts/improve.md`

---

# Recommendations — Develop workflow, run 01M0WWKAQCWZC0Q0JK019H0ZC7

Baseline (from run events and conclusion): clean 6/6 success, 293s wall / 248s active, $0.271, 0 retries, 488k of 566k tokens cache-read. The workflow is healthy; the items below are the seams this run actually exposed.

## 1. Evidence diff context: `-U1` → `-U3` in `evidence.nu`
The reviewer's journal (`.fabro/journal/01M0WWKAQCWZC0Q0JK019H0ZC7.jsonl`, reviewer@1) records: "The -U1 diff context omits where start/last are computed in run(); trusted the green gate plus the pinning tests rather than re-deriving the range math." That is an approval granted on trust at exactly the seam review exists to cover. Change the one `git diff -U1` in `.fabro/workflows/develop/scripts/evidence.nu` to `-U3`; the capture is blob-ref'd anyway (22.9 KB), so size is not binding. Expected effect: range/validation logic becomes verifiable in-diff, removing the "trusted the tests" class of approval.

## 2. Kill the recurring evidence blob-detour: raise `preamble_budget_kb` 24 → 32 (or make it per-value)
The graph comment in `workflow.fabro` says the 12→24 raise was specifically to stop the capture being blob-ref'd — yet in this run the reviewer's preamble still showed `Output (22.9 KB; full value: …/blobs/3aedadc…json)` (from run events, reviewer stage prompt), because the budget is aggregate and 22.9 KB + tester output + context keys exceeded 24 KB. Here it cost only one tool round trip (~100 ms), but run 01M0SS23MJ turned the same detour into a wrongful rejection. Context usage peaked at 1.6% of the 1M window — the window is not the constraint. Expected effect: evidence arrives inline; the Verification-blocked-on-blob failure mode disappears.

## 3. Resolve the "gate green" contradiction between planner brief and implementer rules
The planner's brief (run events, planner@1 context) ends with acceptance bullet "gate green: gofmt, go vet, go build, go test", while the implementer prompt (rule 4) forbids running anything that "formats, lints, or runs the full suite." Result: the implementer ran `gofmt -l; go vet && go test` twice plus a full build (run events seq 162–183) so it could report PASS on that bullet — a direct rule violation forced by the brief. It was benign here (it warmed the cache; the tester then ran in 1.0 s with `ok gofib (cached)`), but the workflow's own design note says this "wastes a cold cache's tens of seconds." One change, pick one: drop the "gate green" bullet from planner-authored briefs (the tester owns it), or add an explicit PASS-verdict of `DEFERRED (tester owns gate)` to the implementer's per-criterion report contract. Expected effect: the violation stops recurring and the gate runs exactly once on cold caches.

## 4. Delete the stale "planner closes seeds" lines from both prompts
From run events, planner prompt line 1: "You own the tracker: **you close approved seeds**…" and reviewer Approved bullet: "**The Planner will close the seed** and pick the next one" — both contradict the deterministic-Closeout design the same prompts state three paragraphs later (`sd close <id>` is "NEVER yours"). The planner complied with the newer text this run, but the opening sentence is what a skimming model weights highest. Edit `@prompts/planner.md` and `@prompts/reviewer.md` to the closeout phrasing. Expected effect: protects the one invariant (only closeout closes) from prompt-weight drift.

## 5. Mandate `timeout_ms` on shell build/test calls (or raise the default)
Run events seq 162–164: the implementer's chained `cat >> … ; gofmt -l .; go vet ./... && go test` was killed at 10,345 ms (`Termination: timed_out`) on the cold vet; the retry with `timeout_ms: 120000` ran in 716 ms. One wasted call plus a partially-executed heredoc. Add to the implementer prompt's tool guidance: "always pass `timeout_ms ≥ 120000` for build/test commands," or raise the shell default. Expected effect: no more 10 s casualties on first-of-run compiles.

## 6. Prune skill-discovery noise from agent turns
Every planner/implementer/reviewer turn injected ~1.6k tokens listing 35 skills (`agent.skills.discovered`, run events) — including TypeScript courses ("migrate-to-shoehorn", "scaffold-exercises") in a Go repo — and `activated: []` in all three stages. That is ~4–5k tokens of pure noise per session across 16+ implementer turns, diluting the prompt for no use. Scope discovery to `.fabro/skills` (the workflow's own dir) or prune `/storage/.home/skills`. Expected effect: small direct token/cost cut, larger effect on instruction-following density in the implementer's many turns.

## 7. Shorten or parallelize the post-exit tail (~53 s, 18% of wall time)
From run events: exit stage completed 16:43:46; branch pushed 16:43:48; `pull_request.created` only at 16:44:35 (47 s later — PR body generation via `zai:glm-4.7`); run.completed 16:44:39. The run looks "done" for ~53 s while the UI still shows it running. Create the PR at the "Tracker empty" edge in parallel with the final metadata snapshot, or emit an interim event so the user sees "creating PR…". Expected effect: perceived latency drops by ~15–18% for zero workflow change.

## 8. Derive `files_touched` from git, not write-tool interception
Run events, implementer@1 `files_touched`: `[README.md, main.go]` — but the stage diff clearly includes `fib_test.go` (edited via `perl -pi` / `cat >>` through the shell tool, which the tracker doesn't see). Anything downstream consuming `files_touched` (reviewer tooling, dashboards, cost attribution) under-counts. Capture `git status --porcelain` at stage end instead. Expected effect: reliable per-stage change attribution.

## 9. Two small journal/prompt nits
- The implementer journaled `observations: ["none"]` despite hitting real friction (the 10 s timeout, the placeholder-append-then-strip churn that cost ~4 calls / ~40 s of its 150 s). Add "self-inflicted tool misfires count as painpoints" to the journal section — that timeout signal was lost precisely where the improve-loop looks for it.
- The planner ran both `sd ready` and `sd list` (seq 38, 68): `sd ready` can't see blocked-but-open seeds, so "tracker empty" is undecidable from it alone. Amend the planner's sd table: "to confirm Tracker empty, `sd list`'s count is authoritative; `sd ready` misses blocked open seeds." Saves a guilt-edged extra call per terminal pass.

What I could not inspect: the blob contents of the tester/gate output beyond its 290-byte preview, and the PR-body generation step (no per-call events between push and `pull_request.created`), so recommendation 7's root cause is inferred from the 47 s gap rather than observed directly.
