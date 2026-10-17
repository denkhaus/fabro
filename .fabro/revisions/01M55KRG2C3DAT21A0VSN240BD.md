# Revision — run 01M55KRG2C3DAT21A0VSN240BD

- status reviewed: succeeded (191.7s wall, $0.170, zero retries, gates first-pass green)
- review: .fabro/reviews/develop/01M55KRG2C3DAT21A0VSN240BD.md
- seeds filed: none — healthy pass; all findings covered by open seeds or already-open overflows
- balance: 0 non-exempt seeds filed / 0 — no credit this pass (no same-pass stale/superseded closes)
- basis: run 01M55KRG2C3DAT21A0VSN240BD, workflow version 0b5288e246234b2935a5fd1bde2f22824b2a825546cd278c8a50a130d256fd6d, commit ce9a6e171013fb446cb1cc41ea367804c1ba17a5
- revised_at_commit: ce9a6e171013fb446cb1cc41ea367804c1ba17a5 (ADR-0015: engine drift signal for later judgement)

## Findings

**1. Deduplicate the gate's lint tiers** — duplicate_of: fabro-0d77 (open, exactly the same change in `.fabro/workflows/loop/scripts/loop-gate.nu`: lint-nu/prompt-lint run once, duplicated validate-workflows tiers dropped). Effect as filed there: tester ~21s → ~10s, gate output halves.

**2. Fix the never-injected planner tool / absorb the in-flight scan into preflight** — duplicate_of: fabro-d62c (open, capability-affecting; the script-layer alternative — moving the open-PR/non-terminal-run scan into `nu .fabro/workflows/develop/scripts/planner-preflight.nu` — recorded in that seed's scope, ADR-0029-friendly).

**3. Stop feeding the planner the 64-seed firehose** — duplicate_of: fabro-c3b4 (open; preflight's ranked candidate table as primary ordering, `seeds show` only the adjudicated top candidate).

**4. Move the static preflight legend out of runtime context** — overflow-dup: Move the static legend out of planner-preflight routing output (open in .fabro/revisions/01M536GGFVG16CGN8WPK05WYS7.md) — identical theme already open on the ledger; cross-noted there for consolidation with the lane-aware-legend entry in `01M52XXVC22TFSNQ6B34W6V83H.md` when a credit-earning pass files it.

**5. Port the fs_hide/glob_filter footgun lines to the loop planner prompt** — duplicate_of: fabro-6997 and fabro-8d61 (both open, develop-scoped; scope should extend to `.fabro/workflows/loop/prompts/planner.md`, same precedent as last cycle's fabro-a08a drop).

**6. Gate-check enumerated workflow lists in shared-script headers** — overflow (no credit this pass)

- overflow: Gate-check enumerated workflow lists in shared-script headers — add a battery (registered in `.fabro/scripts/battery-runner.nu`, logic in `.fabro/scripts/prompt-lint.nu` or a new check) that cross-checks shared `.fabro/scripts/*.nu` headers enumerating workflow names against `rg -l <script> .fabro/workflows --glob workflow.toml`; effect: the next stale header (this run's defect class, "four" vs six workflows residual from fabro-4cc3) goes RED deterministically at the tester instead of spawning a residual revision seed. Dedupe verified this pass: `seeds search "header"` and `seeds search "battery"` show no seed covering this check (fabro-c0c2 is per-lane stage-journal copies, fabro-0d16 unrelated marker-drift); no open overflow names it.

Analyst/selection journal painpoints (Ask-Fabro 401s on seeds search; select→analyze and bookkeeper context render gaps) — duplicate_of: fabro-d6de (in_progress; implementation afef92d43 landed via PR #426 yet this pass's ask still 401'd all seven searches — acceptance evidence it is not yet effective) and fabro-c5a0 (open).

Not acted on (per review): `claim_check` fail-open on a comment-only diff is benign; PR #429 closeout/notify worked as designed.
