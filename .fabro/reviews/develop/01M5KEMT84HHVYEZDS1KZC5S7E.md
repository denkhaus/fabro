# Improve review — run 01M5KEMT84HHVYEZDS1KZC5S7E

- workflow: develop
- branch integrated: this revisor pass (unmerged until approved)
- status: succeeded (111.5s wall, revisor pass — reason and cost in run detail)
- generated: 2026-10-23 06:18+0200 by revisor `fabro_ask`

---

All evidence is gathered. Here are the recommendations, ordered by expected impact. Every claim about what happened is from this run's event stream (stage outputs, tool-call transcripts, timings, cost); the closeout lines are from workspace file `.fabro/workflows/develop/scripts/closeout.nu`; tracker items are from live `fabro_seeds_search`.

**Run baseline (for sizing):** wall 111.5s, total cost $0.1096 — planner 58.4s/$0.084, reviewer 18.7s/$0.025 (from run conclusion). This was a verification-only pass on seed fabro-30a5 ("Action: none"): planner → evidence → reviewer (Approved) → closeout → exit, green throughout.

---

**1. Closeout parks every approved verification-only seed — fabro-30a5 can never leave the queue (livelock).**
- Evidence: reviewer approved (`review_verdict: approved`, "seed is complete as specified and ready for deterministic Closeout"), yet closeout output was `closeout: PARK — ($seed_id) NOT closed: the run diff … shows no implementing change … Seed left OPEN`. That is correct per the gate at `closeout.nu:531-540` (`seed-demand-visible`, fabro-02c4) — but a verification-only pass *by design* produces no implementing diff (the brief says "make NO changes"), so closeout will park it on every pass. The seed stays open/assigned to `loop`, the tracker_guard requeue arm will recycle it, and the next run re-claims it: ~2 min and ~$0.11 burned per cycle, forever, on a seed whose sole criterion was already proven. (The PARK banner also rendered the literal placeholder `($seed_id)` instead of `fabro-30a5` — same message path, fix together.)
- Change: in `.fabro/workflows/develop/scripts/closeout.nu` (`seed-demand-visible` / PARK gate, lines 167-174 and 531-540), treat a reviewer-approved verification-only pass as demand-visible — e.g. have the planner stamp a context key (`verification_only: true`) alongside `current_seed_id` when routing the Verification-only edge, and let closeout read it via stdin/context the same way it reads the seed id.
- Expected effect: verification-only seeds close on their one approving pass instead of re-entering the queue every run; removes an unbounded repeat cost per no-op seed.
- Item: **new item** — searches `closeout verification-only park` (0 hits), `closeout no implementing diff` (0), `closeout park` (only fabro-9e2c, which surfaces the park state in notifications — visibility, not close semantics), and the `verification-only` family (fabro-9d26/fb06/f759/578a/b8ed — routing/brief/capture concerns, none about closure).

**2. `fabro_run_search` status-enum trial-and-error cost the planner two failed calls and two extra LLM turns.**
- Evidence: planner called `fabro_run_search` with `status: ["queued","running","awaiting_input"]` → error `unknown run status 'queued'`; retried with `["running","awaiting_input","planning"]` → `unknown run status 'awaiting_input'`; third call bare `{workflow: "loop"}` succeeded but returned 20 full run records it then had to scan itself. Two wasted round trips ≈ 2 LLM turns ≈ ~13s of the planner's 58s (event timestamps 04:00:55→04:01:05).
- Change: implement open seed **fabro-1cde** ("fabro_run_search: unknown-status error must name the valid RunStatusKind values") — the error text should list valid values so the model self-corrects in one turn; optionally add the literal valid status list (or "omit status") to the IN-FLIGHT EXCLUSION step in `.fabro/workflows/loop/prompts/planner.md`.
- Expected effect: eliminates the retry loop on the first in-flight check of every planner pass (~10-15s and ~$0.01 per run).

**3. The planner inlined a 65-row `seeds ready` firehose (9.4KB) that duplicated what preflight had already computed.**
- Evidence: preflight (3.9s, zero tokens) already returned the 5 top candidates with verdicts; the planner then ran `seeds ready --assignee loop --limit 200` (9,491 bytes retained, 65 rows) and picked fabro-30a5 — preflight candidate #1. The 60 extra rows were pure context ballast in a 26.9K-token prompt.
- Change: implement open seed **fabro-c3b4** ("Use a top-N sd ready view in the planner instead of the full firehose") in the planner prompt's step 1 / PROJECT_FACTS command table — or go further: when `output.preflight.mode == "checked"` and not degraded, let the planner skip `seeds ready` entirely and adjudicate preflight's candidate list.
- Expected effect: ~9KB less planner context per pass, fewer distraction tokens, one fewer tool round trip; cheaper and faster candidate selection with identical outcome in this run.

**4. Run-title write failed once on the 100-char cap — an avoidable extra LLM turn.**
- Evidence: first `fabro_run_set_title` (117 chars) → `run title must be at most 100 characters`; retry at 83 chars succeeded. One wasted LLM turn (~6s, ~$0.007). The cap exists only in the tool description; the RUN TITLE paragraph in `.fabro/workflows/loop/prompts/planner.md` doesn't carry it.
- Change: add one line to the RUN TITLE paragraph in planner.md: "≤100 characters — the tool hard-rejects longer; compress before the first call," with the accepted title from this run as a worked example.
- Expected effect: one fewer LLM round trip per run; removes the only `fabro_run_set_title` error class seen.
- Item: **new item** — searches `set_title` (0) and `run title 100 characters` (0); fabro-fb19 covers inlining engine facts in *briefs*, not the planner's own tool-call contract.

**5. A PR was opened for a bookkeeping-only diff on a run whose seed wasn't even closed.**
- Evidence: PR #478 was created (pull_request_creation succeeded, model zai:glm-4.7) for a final diff of 2 files +9/−1 — the tracker claim row and the run journal — while closeout simultaneously PARKed the seed as having no implementing change. A human now has a review request containing no reviewable work, and this will recur on every verification-only/parked pass once #1 is fixed.
- Change: gate PR creation on closeout's outcome — skip (or auto-close as bookkeeping-only) when the run diff, with `.seeds` and `.fabro/journal` excluded, is empty; the diff computation already exists in `closeout.nu:169` and can be surfaced as the gate signal.
- Expected effect: zero human-facing no-op PRs from verification-only and parked passes; PR queue carries only runs with reviewable diffs.
- Item: **new item** — search `pull request no implementing change` (0 hits); no open seed addresses PR gating on no-op runs.

---

**Not recommending (checked and rejected):** shallow clone for the 10.5s full-history checkout — basis-commit resolution (`basis_commit_resolves`) depends on deep history per the preflight legend, so the saving isn't worth the risk; and the graph itself behaved well (guards 40-367ms, zero retries, correct verification-only routing), so no graph-redesign recommendation is warranted from this run's evidence.
