# Revision — run 01M1XZCWNBR33BZGHQXNRA80J9

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M1XZCWNBR33BZGHQXNRA80J9.md
- seeds filed: fabro-bbbb — Develop planner: drop reasoning_effort from high to low for first-pass claims; fabro-d5fd — Develop planner: probe unfiltered sd list before parking on an empty assignee-filtered view
- basis: run 01M1XZCWNBR33BZGHQXNRA80J9, workflow version 2232aaf05c7a93b3557b65ef13543486edcf917d0a735f9cb430280339b647a2, commit 6677e200fb8d35e90c2a554bdebe0edc69340394
- revised_at_commit: 6677e200fb8d35e90c2a554bdebe0edc69340394 (ADR-0015: engine drift signal for later judgement)

## Findings

### Develop planner: drop reasoning_effort from high to low for first-pass claims — filed fabro-bbbb

Planner was the most expensive stage of the run (83.6 s wall = 40% of run, $0.080 = 45% of cost,
4,469 of 5,899 reasoning tokens), mostly tie-breaking two High seeds. With the deterministic
assignee filter shipped in fabro-5e09 (PR #38), that reasoning is wasted. Change: set
`reasoning_effort="low"` on the planner node at `.fabro/workflows/develop/workflow.fabro:49`
(keep high semantics for changes_requested re-plans if conditional effort isn't available;
re-measure one run). Expected effect: ~$0.03–0.05 and 20–40 s off every claim pass.

### Develop planner: probe unfiltered sd list before parking on an empty assignee-filtered view — filed fabro-d5fd

The run's implementer smoke test showed `sd ready --assignee nonexistentuser` exits 0 with
"No ready issues." — byte-identical to a legitimately empty pool — so a typo'd or renamed
assignee silently parks the line forever under the new park semantics. Change: in the
FAIL-CLOSED paragraph at `.fabro/workflows/develop/prompts/planner.md:46`, before routing
Tracker empty, run one unfiltered `sd list --limit 5` probe; if open seeds exist but the
filtered view is empty, journal "park: N open seeds, none assigned to fabro" so the terminal
Slack notification shows a cause. Cross-references fabro-5e09 (complementary, not superseding).
Expected effect: converts a silent permanent outage into a visible, diagnosable park.
