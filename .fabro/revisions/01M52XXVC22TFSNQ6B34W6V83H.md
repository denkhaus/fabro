# Revision — run 01M52XXVC22TFSNQ6B34W6V83H

- status reviewed: succeeded (7m33s, $0.48, PR #410, claimed seed fabro-deb5 — healthy pass, clean dup-run-check self-closure)
- review: .fabro/reviews/develop/01M52XXVC22TFSNQ6B34W6V83H.md
- seeds filed: fabro-d62c — loop lane run-level fabro tools enable (capability-affecting, needs-user)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass (no same-pass stale/superseded closes; closeout-race count 0)
- basis: run 01M52XXVC22TFSNQ6B34W6V83H, workflow version absent, commit 14631764f848f589500fbe98a1cca4478ea15b06
- revised_at_commit: 14631764f848f589500fbe98a1cca4478ea15b06 (ADR-0015: engine drift signal for later judgement)

## Findings

**1. Enable run-level fabro tools in the loop workflow** — filed as **fabro-d62c** (capability-affecting per ADR-0019: adds the engine-mediated read-only run tools to the planner's tool surface; labels `needs-user,revision`; implementation awaits explicit user approval). Add `[run.agent] fabro_tools = true` to `.fabro/workflows/loop/workflow.toml`; the per-node pins already restrict every other node. Expected effect: planner step 5 (in-flight exclusion) stops running degraded; the duplicate-claim class (~15 min / ~$0.73 per duplicate per fabro-6b58) becomes preventable. This also rehydrated and consumed the open ledger overflow at `.fabro/revisions/01M43J0MSS6HK1N8396YQAADFT.md:25` (consumed → fabro-d62c). Cross-refs: open fabro-9b84 (both lane halves land coherently), closed fabro-c419 (develop-lane enable), closed fabro-96c6 (enable-vs-name semantics).

**2. Deterministic graph↔schema label-consistency tier in `scripts/validate-workflows.nu`** — overflow to journal (no balance credit this pass):

- overflow: validate-workflows graph↔schema label tier — extend the conductor develop schema probe (or add a sibling) in `scripts/validate-workflows.nu` to parse the routing node's outgoing edge labels from the graph and diff them against the schema enum plus validator `expected_labels`; effect: a renamed label that misses one member of the one-unit (graph/prompt/schema/validator) REDs the tester deterministically instead of depending on an LLM choosing to write a mktemp probe (this run's routing change passed the dot-snapshot tier with zero drift). No open seed covers it (closed fabro-de4d is output-schema routing validation; closed fabro-eeea/fabro-ed4d are adjacent infra).

**3. Stop running every lint 4× per run** — overflow-dup: delete the duplicated lint sections from the loop gate (open in `.fabro/revisions/01M43J0MSS6HK1N8396YQAADFT.md`); this pass's arms complement it: (a) one rule in `.fabro/workflows/loop/prompts/implementer.md` — record each check once, re-run only the affected check after edits; (b) drop the internal lint calls from `scripts/validate-workflows.nu` so the battery-runner is the single lint surface (fabro-eae5's design intent). Effect: ~10–20 s wall + ~19 KB repeated output removed per loop run.

**4. Adjudicate the 23 standing prompt-lint warnings (5 stale date pins)** — overflow-dup: Silence by-design prompt-lint warnings in the qualitygate (open in `.fabro/revisions/01M2XQJC3TWRMJ1128WRXRQYXH.md`); this pass's arm: add a `date-pin-acked: <reason>` marker to `.fabro/scripts/prompt-lint.nu` and refresh/ack the 5 flagged files. Effect: standing advisories drop out of the gate log so a new warning becomes visible.

**5. Lane-aware `envelope_risk` legend in `.fabro/workflows/develop/scripts/planner-preflight.nu`** — overflow to journal (no balance credit this pass):

- overflow: planner-preflight lane-aware envelope_risk legend — render the `envelope_risk` legend text per `--assignee` (flag already exists) in `.fabro/workflows/develop/scripts/planner-preflight.nu` instead of the develop-lane-only wording that instructs the loop planner to do the opposite of the lane's rule; effect: removes a contradictory instruction from every loop planner pass (this run's planner reasoning-traced the miscue before self-correcting), one less failure mode on the lane's core routing decision. Closed fabro-ead4 added the legend with develop wording only; no open seed covers it.

**6. Cap the `seeds ready` firehose** — duplicate_of: fabro-c3b4 (open, @loop) already names the top-N ready view; this run is fresh evidence (67 issues / ~9.8 KB pulled when the preflight had adjudicated the top 5).

**7. Reviewer→implementer minor-fix edge** — duplicate_of: fabro-b659 (open, @loop) already names the edge; this run's mechanical 4-file rename diff is a fresh existence proof of the diff shape it targets.

**8. Port probe discipline to `.fabro/workflows/loop/prompts/implementer.md`** — overflow to journal (no balance credit this pass):

- overflow: implementer probe discipline rule — add to `.fabro/workflows/loop/prompts/implementer.md`: one assertion per shell call; never nest quoted patterns through `sh -c` — use a nu one-liner/probe file; effect: fewer wasted verification rounds and less reviewer adjudication of self-inflicted exit-2s (this run's only tool error was exactly this shape, repaired by an improvised nu probe). Closed fabro-e32c fixed the check-transcript wrapper; open fabro-2904 covers nu `$`-var quoting; neither is the implementer prompt rule.
