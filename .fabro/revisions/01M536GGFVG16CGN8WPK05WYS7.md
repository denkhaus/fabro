# Revision — run 01M536GGFVG16CGN8WPK05WYS7

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M536GGFVG16CGN8WPK05WYS7.md
- seeds filed: none — zero credit this pass (balance gate, ADR-0022)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M536GGFVG16CGN8WPK05WYS7, workflow version absent, commit 8082345c8290cec1ddc127ae55805e058b93e206
- revised_at_commit: 8082345c8290cec1ddc127ae55805e058b93e206 (ADR-0015: engine drift signal for later judgement)

## Findings

Dropped at analyze (live dedupe): `fabro_run_search` absence → duplicate_of fabro-d62c (open, capability-affecting, same change); fresh-cycle line port to loop planner → duplicate_of fabro-a08a (open, assignee loop — scope should extend to `.fabro/workflows/loop/prompts/planner.md`); planner firehose top-N view → duplicate_of fabro-c3b4 (open). Closeout-race re-check at filing time: only closed fabro-1d70 (stranded-revision salvage) cites the run — covers none of the findings. No same-pass stale/superseded closes → credit 0 → all surviving findings journal as overflow for the next pass.

**1. Closeout: park gate contradicts the Verification-only route + park message never names the seed** — overflow (zero credit this pass)
- overflow: Closeout park gate accepts approved verification-only proofs and parks message interpolates the seed id — two arms in `.fabro/workflows/develop/scripts/closeout.nu`: (a) pass route/verdict into closeout (graph's closeout node already has `stdin_source="current_seed_id"`; add the route key or read `review_verdict` from context) and skip the diff-demand check when the planner's recorded-checks section exists and the reviewer approved (~lines 493–505); (b) unescape `\( $seed_id \)` to interpolating `($seed_id)` at line 498 so park notes name the parked seed; effect: approved verification-only seeds actually close instead of parking every cycle (kills a repeat ~2.5-min/$0.16 verification cycle per parked seed per scheduled run and stops no-op PRs like #420), and park notes identify the seed for the next cycle. Cross-ref: open fabro-ab38 (merged-run sibling sweep) is the adjacent problem, not this; open overflow "Surface the nu `$\"…\"` literal-paren trap" (`.fabro/revisions/01M5332MTX1GDRHNS0D9QW2WHK.md`) covers the lint/prevention class — arm (b) is the concrete fix instance.

**2. Parked runs read green: exit label, notification, and PR don't distinguish "parked"** — overflow (zero credit this pass)
- overflow: Parked runs read green — in `.fabro/workflows/loop/workflow.fabro`, have closeout emit a `preferred_label="Parked"` and route it to a distinct exit kind (e.g. `x.kind="soft"`, mirroring the "Closeout failed" edge) instead of the unconditional `closeout -> exit [label="Seed closed"]`; effect: terminal status, Slack notification, and PR reflect the parked state instead of green-washing it, so a human scanning notifications can see the loop spinning on a stuck seed (this run: `succeeded` + `run.completed` to #dev-fabro + no-op PR #420 while fabro-41fa stayed open). Cross-ref: open overflow "Make the terminal notification an actionable park message" (`.fabro/revisions/01M4KMWYRQDJ54ZSS08NJNH74N.md`) is the quota-soft-stop notification payload — different trigger, complementary.

**3. Move the static routing legend out of `planner-preflight.nu`'s per-run output** — overflow (zero credit this pass)
- overflow: Move the static legend out of planner-preflight routing output — relocate the ~1.9 KB static legend from `.fabro/workflows/develop/scripts/planner-preflight.nu`'s routing JSON into the planner prompt (it is documentation, not per-run data); effect: ~2 KB less per-pass planner context on every scheduled run, compounding with open fabro-c3b4's top-N view (the firehose arm of the same recommendation). Cross-ref: open overflow "planner-preflight lane-aware envelope_risk legend" (`.fabro/revisions/01M52XXVC22TFSNQ6B34W6V83H.md`) fixes the legend's wording per lane — different mechanism (content vs location); a future pass may consolidate both as arms of one seed.

Next pass: re-run live dedupe on all three, then file against that pass's balance (consolidate arms 1(a)+1(b) as one multi-arm seed; consider folding finding 3 and the lane-aware legend entry into one preflight-legend seed).
