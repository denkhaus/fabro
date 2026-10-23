# Revision — run 01M5K62554W7YCVZ7W4AT8YYWK

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M5K62554W7YCVZ7W4AT8YYWK.md
- items filed: none — tracker outage (fabro_work_create fails: "no work project is bound to target repository denkhaus/fabro"); one needs-user-exempt re-file attempted, tool call failed
- balance: 0 non-exempt items filed / 0 — no credit this pass
- basis: run 01M5K62554W7YCVZ7W4AT8YYWK, workflow version absent, commit 1c34eadbe14c271599b74497a6052806b74b1a66
- revised_at_commit: 1c34eadbe14c271599b74497a6052806b74b1a66 (ADR-0015: engine drift signal for later judgement)

## Findings

**1. Verification-only seeds can never close — approved work parks forever** — overflow

- overflow: Verification-only approved seeds park forever — add a verification-only arm to `closeout.nu` (lines 167–174: the `seed-demand-visible` patch excludes `.seeds` and `.fabro/journal`, so a verification-only run whose only artifact is the tracker record — stat map `seed +1/-1 .seeds/issues.jsonl` — has structurally invisible demand): when the planner routed `Verification-only` and `review_verdict == approved`, close with `seeds close <id> --reason "verified: <one-line verdict> (run <id>)"`; effect: verified seeds leave the loop queue instead of re-entering it (this run paid $0.238 / 5m05s to re-answer fabro-30ae a future run would re-pay again). Dedupe: fabro-9d26 (closed, fast path), fabro-f759/fabro-d89a (closed, evidence capture), fabro-02c4 (mandates the park discipline, opposite direction) — none closes verified seeds.

**2. Planner 67-row firehose** — duplicate_of: fabro-c3b4 (open; this run is another live occurrence, 9.8KB / 67 rows at event seq 92–95 despite preflight's inlined top-5). Evidence belongs to fabro-c3b4's implementer.

**3. Title instruction "a few sentences" contradicts the 100-char tool cap** — duplicate_of: fabro-b529 (open; scope explicitly includes the loop-lane planner title-instruction arm; this run: 3 `fabro_run_set_title` calls, 2 errors over cap).

**4. Evidence capture hides tracker-only seed work from the reviewer** — overflow

- overflow: Evidence capture hides tracker-only seed work — in `.fabro/workflows/develop/scripts/evidence.nu`, when seed-work is tracker-only, echo the changed tracker record's fields (status flip, appended note) in place of the empty diff body ("zero per-seed hunks … no diff body by design" forced this run's reviewer into 4 independent grep re-runs to adjudicate a +1/-1 stat); effect: reviewer judges recorded content instead of re-deriving it. Dedupe: fabro-e042 (marker for zero-hunk files), closed fabro-5e62 (hard-error on empty diff) — none renders tracker-record content.

**5. Loop reviewer node cannot read the evidence blob** — overflow (needs-user; ADR-0019)

- overflow: Loop reviewer node cannot read the evidence blob — capability-affecting per ADR-0019, two arms: (a) grant the loop reviewer node a read-only blob-read tool via `x.fabro_tools` in `.fabro/workflows/loop/workflow.fabro` (adds a tool to an agent-reachable allowlist — implementation awaits explicit user approval), or (b) non-capability alternative: `evidence.nu --lane loop` inlines a capped plain-text capture (develop-lane analogue: closed fabro-8cef); effect: reviewer judges the captured evidence instead of falling back to the working tree (basis occurrence run 01M5JRAP1F67RZE674TBRPGPY8: 100,765-byte blob the reviewer could not read). `fabro_work_create` with labels `["needs-user","revision"]` attempted this pass and failed on the tracker binding outage — retry next pass under the needs-user exemption.
- overflow-dup: Loop reviewer node cannot read the evidence blob (open in 01M5JRAP1F67RZE674TBRPGPY8.md line 16 — linked, not re-filed)

**6. Resolve workflow-relative shorthand paths in claim-check/preflight** — still-open ledger entry from a prior pass (link only, not filed this pass)

- overflow-dup: Resolve workflow-relative shorthand paths in claim-check/preflight (open in 01M5JRAP1F67RZE674TBRPGPY8.md line 13 — linked, not re-filed)

**7. Work-project tracker binding outage (revisor tooling dead)** — overflow

- overflow: Bind the work project tracker to this repository — server-side, bind the work project's targetRepo to `denkhaus/fabro`, or name the project id in the revisor FACTS block (the `fabro_work_*` tools' project id is omitted, so every call fails with "no work project is bound to target repository denkhaus/fabro"); effect: revisor filing/dedupe returns to engine-mediated tracker tools instead of degraded `.seeds/issues.jsonl` greps. Outage now spans all stages of revisor run 01M5K208R20CB3007RH84H39NM plus both cycles of 01M5K625555R46VT94D1XGXENC, surviving the target run's own landing of PR #468 (commit 9c8139300) — the outage is server-side.

**8. Staged FACTS merge-target ref `origin/denkhaus` is unfetchable** — overflow

- overflow: Re-pin the staged FACTS merge-target ref — the revisor FACTS value `origin/denkhaus` is absent from this workspace (third consecutive pass), degrading merge-target adjudication to a `git log --all` trailer grep; either fetch/mirror the ref into revisor workspaces or repin the FACTS to a fetchable ref; effect: duplicate-run adjudication becomes the designed ref check instead of a degraded grep. Cross-ref: open fabro-58cb names the same branch in develop PROJECT_FACTS (thematic overlap, different surface — not supersession).

Dropped as duplicates at analyze time (verified open this pass, degraded id lookup): rec 2 → fabro-c3b4, rec 3 → fabro-b529, rec 5 → fabro-6997 (`rg -r` footgun fired live again this run), rec 6 → fabro-9e2c (the `($seed_id)` literal at `closeout.nu:536` is prerequisite plumbing inside fabro-9e2c's park-notice path). Step 3.6 prompt-lint: not applicable (run diff was tracker-only, +1/-1 on `.seeds/issues.jsonl`).
