# Improve review — run 01M5K62554W7YCVZ7W4AT8YYWK

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (03:30–03:35 Berlin on 2026-10-23, 5m05s, $0.238, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 04:15+0200 by revisor `fabro_ask`

---

# Run 01M5K62554W7YCVZ7W4AT8YYWK — improvement recommendations

**What this run was** (from run projection + events): a scheduled `loop` pass that claimed seed **fabro-30ae** ("verify engine-side whether stdin_source can resolve current_work_item"), routed **Verification-only** (planner → evidence → reviewer → closeout → exit), answered the question (verdict: no), reviewer **Approved** — and then **closeout PARKed the seed it couldn't name**, the run exited **green via the edge labeled "Seed closed"**, and PR [#470](https://github.com/denkhaus/fabro/pull/470) opened anyway. Totals: 5m05s wall, $0.238 cost, planner = $0.196 / 216s of that (82% of cost, 71% of wall). Live tracker confirms **fabro-30ae is still `in_progress`** — the next run will re-claim and re-verify it.

Tracker searches I ran to dedup (all via `fabro_seeds_search`): `"verification-only closeout"` (0), `"closeout stdin"` (0), `"closeout"` (13), `"verification"` (26), `"evidence"` (45), `"evidence tracker record"` (0), `"park"` (23), `"title"` (13), `"set_title"` (0), `"6997"` (1), `"30ae"` (1), plus a full open-queue listing (`assignee=loop`, `status=open`, 66 matched).

---

**1. Closeout has no closure arm for approved verification-only seeds — they park forever and the exit edge lies.** (highest impact — systemic)
- **Evidence:** closeout PARKed despite `review_verdict=approved` in final context (run events, invocation.finished seq 452); the graph's unconditional `closeout -> exit [label="Seed closed"]` edge carried the park out green (seq 440); tracker shows fabro-30ae still in_progress. The park logic (closeout.nu:118–174, fabro-02c4 discipline) requires an implementing diff — which a verification-only run *by definition* never has.
- **Change:** in `.fabro/workflows/develop/scripts/closeout.nu` add an approved-verification arm (close with reason citing the run + verdict row when `review_verdict=approved` and the only seed-work file is the tracker record); in `.fabro/workflows/loop/workflow.fabro` give the `closeout` node `output_schema="routing"` and split the exit edges into `Seed closed` / `Parked` / `failed` so a park can't ride the "Seed closed" edge.
- **Expected effect:** approved verification seeds actually close; each currently re-verified lap costs ~$0.24 + 5 min + a PR; run terminal state stops reporting "Seed closed" for an open seed.
- **Seed:** *new-seed justification:* no open seed covers closure semantics for the verification-only path — `fabro-9d26` (closed) built the fast path without a closing arm, and open `fabro-9e2c` covers only surfacing the park state in the terminal event/notification.

**2. Reviewer adjudicated a tracker-only diff blind.**
- **Evidence:** evidence capture printed "(zero per-seed hunks … by design)" for `.seeds/issues.jsonl +1/-1`; the reviewer journal logged verbatim: "the tracker delta itself is unreadable from the capture … the reviewer must adjudicate a +1/-1 stat with no visible content" (run events, reviewer stage output).
- **Change:** in `.fabro/workflows/develop/scripts/evidence.nu`, when the per-seed patch is tracker-only, echo the changed record's fields (status + verdict line) instead of the empty-hunks placeholder.
- **Expected effect:** reviewer judges the actual verdict row rather than trusting the stat map — faster, defensible approvals on every verification-only run.
- **Seed:** *new-seed justification:* searches `"evidence tracker record"` and `"evidence"` found no open seed on tracker-record readability (closed `fabro-5e62`/`fabro-31f7` fixed different capture gaps).

**3. The PARK message never names the seed — literal `($seed_id)` in the output.**
- **Evidence:** closeout stdout (run events, step.finished seq 432) reads "PARK — **($seed_id)** NOT closed" while the same final context holds `current_seed_id: fabro-30ae`. From workspace file `.fabro/workflows/develop/scripts/closeout.nu:536`: the message is built as `$"…\($seed_id\)…"` — the escaped parens suppress Nushell interpolation, so it *always* prints the placeholder. (The doubled line is intentional — stdout+stderr, lines 537–538.)
- **Change:** unescape at closeout.nu:536 to `($seed_id)`.
- **Expected effect:** park notes become actionable for the next run and for humans reading journal/notifications; one-line fix.
- **Seed:** *new-seed justification:* searches `"closeout stdin"` and `"closeout"` found no seed for this placeholder bug.

**4. `rg -r` replace-flag footgun hit live, corrupting the planner's first probe.**
- **Evidence:** planner journal (run events): "`rg -rn` is replace-flag `-r n`, not recursive-nth; it silently rewrote every match to 'n' in my first petri probe this run … Reinforces open seed fabro-6997; suggest also covering `rg -r`." Grep of workspace prompts shows **neither** `.fabro/workflows/develop/prompts/planner.md` **nor** `.fabro/workflows/loop/prompts/planner.md` (the one this run used) mentions it.
- **Change:** when implementing **fabro-6997** (open, assignee loop), cover the whole `-r` replace-flag family and land the one-liner in **both** planner prompts — loop's own prompt is unprotected.
- **Expected effect:** no silently rewritten probe output; recorded checks can't be founded on corrupted matches. — **Seed: fabro-6997.**

**5. Run-title attempts burned two rejected calls; the 100-char cap vs 125-char seed title.**
- **Evidence:** planner tool stats: `fabro_run_set_title` 3 calls / **2 errors** (run projection); the first title event (seq 20) shows the generic boilerplate; the accepted title is a 76-char compression of a 125-char seed title.
- **Change:** the relief open seed **fabro-b529** asks for (raise the cap or auto-trim), or as a stopgap one line in `.fabro/workflows/loop/prompts/planner.md`: "set_title must be ≤100 chars — compress the seed title."
- **Expected effect:** correct title on first call; ~2 fewer wasted planner calls per claim. This run is fresh loop-lane evidence for that seed. — **Seed: fabro-b529** (open).

**6. Reviewer re-executed all four recorded checks the evidence already inlined.**
- **Evidence:** reviewer made 4 grep calls reproducing the planner's greps verbatim ("independently re-verified all four recorded checks"), 54.3s / $0.042 of a $0.238 run (run projection stage stats).
- **Change:** in `.fabro/workflows/loop/prompts/reviewer.md`, for Verification-only runs judge from the recorded check transcript and re-run at most the single decisive check (here: the exit-1 attractor grep).
- **Expected effect:** roughly halves reviewer stage time/cost on verification passes, consistent with closed fabro-50c9's intent.
- **Seed:** *new-seed justification:* open `fabro-17fb` dedupes the tester-section-vs-evidence duplication, not planner-check re-execution on the verification-only path.

**Not inspectable here:** per-message planner prompt transcripts and the PR-body generation call, so planner-side token waste beyond the observed tool-call counts can't be attributed further.
