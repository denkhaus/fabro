# Revision — run 01M2MJ0XJ0RH0RGMDARDD2K37X

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2MJ0XJ0RH0RGMDARDD2K37X.md
- seeds filed:
  - fabro-9ba9 — Exclude .mulch from the closeout seed-demand-visible patch
  - fabro-9e2c — Surface the closeout park state in the terminal event and notification
- basis: run 01M2MJ0XJ0RH0RGMDARDD2K37X, workflow version 2dee3f51834119fc062aef7c1ae6de5aa3010cc0024f21ad21923d3c8ac5c523, commit ae3940858bd26c6a8104ac44728d378e0c31db53
- revised_at_commit: ae3940858bd26c6a8104ac44728d378e0c31db53 (ADR-0015: engine drift signal for later judgement)

## Findings

### Exclude .mulch from the closeout seed-demand-visible patch (fabro-9ba9)

The seed-demand-visible diff in `.fabro/workflows/develop/scripts/closeout.nu` excludes only `.seeds` and `.fabro/journal`, but `.mulch/expertise/workflows.jsonl` gained a record quoting the seed title verbatim in this run — so a churn-only mulch edit can satisfy demand visibility and close a seed with no implementation (the fabro-9967 false-close class). Fix: add `:(exclude).mulch` to the pathspec. Expected effect: the pre-close gate can no longer be satisfied by mulch record churn. Searches for the mulch-exclusion theme found no existing seed (fabro-8a60 is qualitygate path verification, fabro-6db3 is evidence.nu churn classification — different mechanisms).

### Surface the closeout park state in the terminal event and notification (fabro-9e2c)

This run introduced the closeout park branch (`closeout.nu` parks with exit 0, seed stays open). A parked run exits succeeded/natural, indistinguishable from a real close in the terminal event and Slack payload; the implementer journal flags the green non-closing closeout interaction as untested. Fix: emit a park marker in `closeout.nu` and surface "seed parked (open)" in the terminal event/notification (or route a soft exit). Expected effect: the user and next planner see the hold without grepping the journal. Adjacent seeds verified different (fabro-b1d3 Slack annotation for bookkeeping-only runs, fabro-5b0a planner fail-open warnings).
