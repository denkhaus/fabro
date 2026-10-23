# Revision — run 01M5KQ7FB5CZ9JHATY12SC7ASS

- status reviewed: blocked
- review: .fabro/reviews/develop/01M5KQ7FB5CZ9JHATY12SC7ASS.md
- items filed: none — healthy run blocked by server-side tracker binding outage (no claim/grant, no diff, no PR)
- balance: 0 non-exempt items filed / 0 — no credit this pass (tracker tools unusable: every `fabro_work_*` call fails "no work project is bound to target repository denkhaus/fabro"; no close is possible, no close earned credit)
- basis: run 01M5KQ7FB5CZ9JHATY12SC7ASS, workflow version 36a97a61355a3299e897ff930fcdb9f12bf8ebedcdee1ccf69f0c80cb51f17ed, commit f8991305d2ec3fdead8737e1e7dd6434a6a2a7fd
- revised_at_commit: f8991305d2ec3fdead8737e1e7dd6434a6a2a7fd (ADR-0015: engine drift signal for later judgement)

## Findings

**1. Pause the loop-fabro schedule until the tracker epic's Phase 6 GO** — overflow

- overflow: Pause the loop-fabro schedule until the tracker epic's Phase 6 GO — server-side (mirtuell.net), disable the `loop-fabro` automation schedule (or bind the work project's targetRepo to `denkhaus/fabro`) until epic fabro-a33e Phase 6 GO; effect: zero further guaranteed-dead scheduled fires, each currently burning ~$0.059/~50s and parking on a human interview (this run and #487 prove the pattern; the migration doc mandates the line stay OFF until Phase 6 + user GO). Needs-user (server configuration).

**2. Unattended-interview routing rule in the loop planner** — overflow

- overflow: Unattended-interview routing rule in the loop planner — in `.fabro/workflows/loop/prompts/planner.md`, add: "unattended schedule fire + no interview channel configured → route `Blocked`, don't `request_user_input`"; effect: a silently parked run becomes a Slack-visible blocked run instead of an invisible pending question (this run: `interviews.provider = null`, engine Slack events are only started/completed/failed, so nobody was pinged). Cross-ref: open fabro-07cf (per-scope interview channel) is the complementary channel-config half; `notification blocked` searches returned 0.

**3. Trim the preflight payload inlined into the planner** — overflow

- overflow: Trim the preflight payload inlined into the planner — in `.fabro/workflows/develop/scripts/planner-preflight.nu`, emit null-suppressed candidate rows and move the repeated advisory `legend` out of `context_updates` (keep it in `--help`); effect: ~1.5KB / ~400 tokens saved per planner pass and less misread risk, no behavior change (evidence: preflight stdout 3,590 bytes, roughly half legend + ~20 null fields per candidate, inlined verbatim into the planner prompt; fabro-ead4 which added the legend is closed, nothing open covers slimming it).

Duplicate withdrawals (not overflow — covered by existing items):

- rec-1 binding half and the select painpoint → duplicate_of open overflow "Bind the work project tracker to this repository" (01M5K62554W7YCVZ7W4AT8YYWK.md line 35, epic fabro-a33e).
- rec-2 tracker_guard fail-closed and rec-3 planner binding-error fail-fast → duplicate_of open overflow entries 1–2 in 01M5KNGHH0ZAPKSD07RX7N8CKS.md (prior pass distilled the same changes on the same outage; linked, not re-filed).
- rec-4 zai error-flag drop → duplicate_of open fabro-c98c (WARN-spam half fabro-e486).
- rec-5 channel-config half → duplicate_of open fabro-07cf.
