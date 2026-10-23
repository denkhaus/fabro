# Improve review — run 01M5K62554W7YCVZ7W4AT8YYWK

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (5m05s wall / $0.238, planner 216s / $0.196 = 82% — verification-only pass on fabro-30ae; revisor pass — reason and cost in run detail)
- generated: 2026-10-23 03:49+0200 by revisor `fabro_ask`

---

All evidence is in. Here are the recommendations, ordered by expected impact, each grounded in this run's events (run `01M5K62554W7YCVZ7W4AT8YYWK`, verification-only pass on `fabro-30ae`: 305s wall, $0.238 total, planner = 216s / $0.196 = 82% of cost).

Tracker searches I ran to dedupe: `verification-only`, `closeout`, `title`, `evidence tracker`, `evidence capture`, `footgun rg`, `firehose ready`, `park seed open approved`.

---

**1. Verification-only seeds can never close — approved work parks forever** *(graph/script design)*
**What happened:** reviewer returned `Approved` (`review_verdict: approved`, from run events), yet closeout printed `PARK — … shows no implementing change … Seed left OPEN` and the run exited green. Cause: `closeout.nu`'s `seed-demand-visible` (lines 167–174, workspace file `.fabro/workflows/develop/scripts/closeout.nu`) excludes `.seeds` and `.fabro/journal` from the patch — but a verification-only run's *only* artifact is the tracker record (`stat map: seed +1/-1 .seeds/issues.jsonl`). Demand is structurally invisible on this route, so every approved verification-only seed re-enters the queue and a future run re-pays ~$0.24 + 5 min to re-answer `fabro-30ae`.
**Change:** add a verification-only arm to `closeout.nu` (shared script, fires for `--lane loop` and develop): when the planner routed `Verification-only` and `review_verdict == approved`, close with `seeds close <id> --reason "verified: <one-line verdict> (run <id>)"`.
**Expected effect:** verified seeds actually leave the loop queue; eliminates whole duplicated runs.
**Seed:** none covers this — searched `verification-only` (7 hits: fast-path fabro-9d26, evidence fabro-f759/d89a — all closed, none about closure), `closeout` (13), `park seed open approved` (0). *New-seed justification: no open or closed seed addresses the closeout park-vs-verified-closure lifecycle gap for the verification-only route.*

**2. Stop the planner's 67-row firehose** *(tool usage / cost)*
**What happened:** the planner's first shell call was `seeds ready --assignee loop --limit 200` → 9.8KB, 67 rows (event seq 92–95), even though the preflight node had *already* inlined the top-5 candidates in its context. The planner is 71% of wall time and 82% of run cost (21 messages, 25 tool calls).
**Change:** implement the top-N ready view per **fabro-c3b4** (open, @loop, `{revision}`), or as the prompt-side stopgap teach `.fabro/workflows/loop/prompts/planner.md` to treat the preflight candidate table as the primary candidate source and skip the full listing when present.
**Expected effect:** removes a ~10KB context flood per planner visit; fewer turns → direct cut into the 216s/$0.196 planner stage.

**3. Title instruction contradicts the 100-char tool cap** *(prompting)*
**What happened:** `fabro_run_set_title`: 3 calls, **2 errors** (from run events, planner `agent.tools`). The prompt says "refine the title to **a few sentences** specific to that seed"; the tool "rejects titles over 100 characters." The eventual 76-char one-liner succeeded — the wording guarantees cap rejections.
**Change:** in `.fabro/workflows/loop/prompts/planner.md` (and develop's twin), change "a few sentences" to "one line, ≤100 characters" — the loop-lane half of **fabro-b529** (open: "Run-title 100-char cap too tight for the ADR-0030 instruction").
**Expected effect:** eliminates 1–2 wasted tool calls per run and the risk of a titleless kanban tile.

**4. Evidence capture hides tracker-only seed work from the reviewer** *(error handling / review UX)*
**What happened:** evidence printed "(zero per-seed hunks … listed above with no diff body by design)" for the only artifact; the reviewer's journal painpoint (from run events): it "must adjudicate a +1/-1 stat with no visible content … A one-line echo of the tracker record's changed fields would remove the need to trust the stat map blindly." The reviewer compensated with 4 independent grep re-runs.
**Change:** in `.fabro/workflows/develop/scripts/evidence.nu`, when seed-work is tracker-only, echo the changed tracker record's fields (status flip, appended note) in place of the empty diff body.
**Expected effect:** reviewer judges recorded content instead of re-deriving it — fewer reviewer tool calls, faster approvals.
**Seed:** none — searched `evidence tracker` (0) and `evidence capture` (12; fabro-5e62 closed, covered empty-diff *hard-error*, not record echo). *New-seed justification: no existing seed covers rendering tracker-record content in the evidence capture.*

**5. The `rg -r` footgun fired live, again** *(prompting / correctness)*
**What happened:** planner journal painpoint (from run events): "`rg -rn` is replace-flag `-r n` … it silently rewrote every match to 'n' in my first petri probe **this run**" — matching the 1 shell error in 20 calls. A silently rewritten probe can flip a verification verdict.
**Change:** extend **fabro-6997**'s (open, @loop) second one-liner from "never `rg -rn`" to cover `rg -r` in general, in `.fabro/workflows/loop/prompts/planner.md` (footgun list) — exactly what the planner itself suggested.
**Expected effect:** kills a recurring silent-output-corruption class in the stage that renders verdicts.

**6. Park note prints a literal `($seed_id)` placeholder** *(script bug, one line)*
**What happened:** closeout output (run events) shows `PARK — ($seed_id) NOT closed` — the literal placeholder, printed twice (stdout + stderr by design, closeout.nu line 128 comment). Cause: `closeout.nu:536` uses `\$($seed_id)\` — the backslash-escaped parens suppress nushell interpolation.
**Change:** drop the backslashes at `.fabro/workflows/develop/scripts/closeout.nu:536` so `$seed_id` interpolates.
**Expected effect:** park notices (and anything building on **fabro-9e2c**, open: "Surface the closeout park state in the terminal event and notification") name the actual seed — this fix is prerequisite plumbing for fabro-9e2c, so file it under that seed.

---

**What I could not inspect:** the PR #470 body/CI results and the planner's full 20 shell-call transcript beyond the sampled events (event stream paged at 100 of ~319K); per-tool error text for the 2 title rejections is not stored in the projection, only call/error counts.
