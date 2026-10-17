# Revision — run 01M550CAXV85JGT1XBPCDRE5DY

- status reviewed: succeeded (loop-lane run, revisor pass)
- review: .fabro/reviews/develop/01M550CAXV85JGT1XBPCDRE5DY.md
- seeds filed: none — healthy pass plus overflow-only findings (zero credit, ADR-0022)
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M550CAXV85JGT1XBPCDRE5DY, workflow version absent, commit a77e71a1ca63135346d72359e3f3c7f4b27afe71
- revised_at_commit: a77e71a1ca63135346d72359e3f3c7f4b27afe71 (ADR-0015: engine drift signal for later judgement)

## Findings

Run verdict: healthy pass — seed `fabro-899b` was closed by the run's own PR #423 (commit `29cac4196`, Fabro-Run trailer names the run; git-verified self-closure). The `duplicate` verdict from `dup-run-check.nu` was a proven false positive (substring match hit `fabro-2366`'s close line whose title text contains the string `fabro-899b`) — that defect is overflow finding 1 below.

Duplicate drops (no filing claim):
- duplicate_of: fabro-d62c — enable run-level fabro tools (`[run.agent] fabro_tools = true` in `.fabro/workflows/loop/workflow.toml`) so the planner's in-flight check stops running degraded.
- duplicate_of: fabro-0d77 — delete the duplicated lint tiers inside `scripts/validate-workflows.nu`.
- duplicate_of: fabro-d19e — nushell-scripts skill to cut implementer debug cycles.
- covered_by_closeout: fabro-2366 — develop-fixture isolation between parity-battery teeth (filing-time re-check confirmed; seed closed, change landed via #424).
- duplicate_of: fabro-c3b4 — loop-lane top-N ready view (scope extension of the develop half, consistent with visit-1's drop).

Overflow findings (survived dedupe; zero balance credit this pass, ADR-0022 — next pass may re-file after re-running its own dedupe):

**1. Anchor dup-run-check's seed-id match to the id field** — overflow (zero credit this pass)
- overflow: Anchor dup-run-check's seed-id match to the id field — in `.fabro/scripts/dup-run-check.nu` (closing-seeds-commit), match the candidate id against the `"id":"<seed>"` JSON field instead of a raw substring of the diff line, so a title merely containing another seed's id string cannot flip the verdict; effect: eliminates false `duplicate` verdicts like this pass's (fabro-899b self-closure misread via fabro-2366's close line), which would have parked a healthy run as a duplicate.
- No duplicate: `seeds search` on dup-run-check shows fabro-679a (counting landed commits, different mechanism) and fabro-7ce7 (module taxonomy) — neither covers id anchoring. No open overflow on this theme.

**2. seeds list compact must not silently omit Medium-priority open seeds** — overflow (zero credit this pass)
- overflow: seeds list compact must not silently omit Medium-priority open seeds — in the seeds CLI, make `seeds list --format compact` cover all priorities (or cap rows per priority, not overall), so the revisor Step 3 tracker picture includes the priority class where loop-hygiene seeds live; effect: dedupe stops missing open Medium seeds (this pass: 50 rows all Critical/High while fabro-0d77, fabro-a23f, fabro-c3b4, fabro-b769 are open Medium, recoverable only via id-verified `seeds show`).
- No duplicate: `seeds search` on list/Medium shows only unrelated handoff seeds. No open overflow on this theme.

**3. Select stage should overwrite revisor_target_* context fields** — overflow (zero credit this pass)
- overflow: Select stage should overwrite revisor_target_* context fields — make the revisor select stage write the selected run id into `revisor_target_run_id`/status/version context (or have the bookkeeper prompt read the target from the journal's select entry), so the marker target cannot diverge from the reviewed run; effect: prevents stale-target bookkeeping like this run's visit-1 (context carried parked run 01M5527Z3G61253S8QCVG5XN65V while select/analyze targeted 01M551H7GZB15GG88D05PA551V).
- No duplicate: `seeds search` on revisor_target shows fabro-9711 (run titles) — different mechanism. No open overflow on this theme.

**4. Never file seed bodies citing dead tmp/ evidence anchors** — overflow (zero credit this pass)
- overflow: Never file seed bodies citing dead tmp/ evidence anchors — in the arch-gate filing path / revisor candidate filing, require measured evidence to be inlined into the seed body (never a `tmp/` path citation), or make preflight's `missing_file` anchor auto-file a body-fix task at claim time; effect: removes the recurring pre-claim correction loop (one tracker write + one LLM adjudication per affected seed — hit by fabro-899b and still pending on fabro-0d77) and clears advisory noise from downstream claim_check.
- No duplicate: fabro-d20f governs brief formatting of corrections after a dead path is detected, not filing-time anchor hygiene; `seeds search` on anchor shows no covering seed. Distinct from the open "sweep stale path anchors in open seed bodies" overflow (`.fabro/revisions/01M43J0MSS6HK1N8396YQAADFT.md`) — that fixes two specific bodies, this changes the filing rule.

**5. Scope implementer pre-gate checks to the diff, not the whole repo** — overflow (zero credit this pass)
- overflow: Scope implementer pre-gate checks to the diff, not the whole repo — in `.fabro/workflows/loop/prompts/planner.md` (Envelope note), prescribe single-file parse probes for new/edited scripts (e.g. `nu -c "source <file>"`) and leave whole-repo lint/battery tiers exclusively to the tester node; effect: removes double execution of every lint/battery per run (this run: full 21.6s gate battery run by both implementer and tester) and trims the most expensive stage's transcript.
- No duplicate: `seeds search` on pre-gate/single-file returns zero hits; fabro-0d77 dedupes tiers inside the gate only. Distinct from the open "Tier the Loop Gate for prompt-only diffs" overflow (`.fabro/revisions/01M5359XETC13K71APW26CQQR3.md` — gate tiering, different file/mechanism) and the implementer probe-discipline overflows.
