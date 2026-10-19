# Revision — run 01M59JSC3MKGRCH5PMR53XHADZ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M59JSC3MKGRCH5PMR53XHADZ.md
- seeds filed: none — zero balance credit this pass (0 same-pass stale/superseded closes); 4 surviving findings journaled as overflow, 2 linked to open overflows, 3 dropped as duplicates
- balance: 0 non-exempt seeds filed / 0 — no credit this pass
- basis: run 01M59JSC3MKGRCH5PMR53XHADZ, workflow version fb1f7f607b87f1d7a46429700779b96154992a343bd3240cd95aa39120df9928, commit 8cdb508e7e9970e00c306acb82d8060b17be20ed
- revised_at_commit: 8cdb508e7e9970e00c306acb82d8060b17be20ed (ADR-0015: engine drift signal for later judgement)

## Findings

Duplicates (dropped, no filing claim):

- Planner firehose trim — duplicate_of: fabro-c3b4 (open; extended body covers top-N `seeds ready` view + `created_since`; sibling fabro-6b58 covers self-exclusion)
- Trailing-newline lint check — covered_by_closeout: fabro-6b0f (and fabro-fad5), both first-party from this run's closeout; filing-time closeout-race re-check ran clean (count=2, no new coverage of other findings)
- Long-stage heartbeat — duplicate_of: fabro-b769 (open)

Surviving findings, no balance credit — overflow:

**1. Filter anecdotal path citations in seed-spec-paths** — overflow

- overflow: Filter anecdotal path citations in seed-spec-paths — in the shared spec-path extractor (fixtures in `.fabro/scripts/seed-spec-paths-fixtures.nu`), drop paths cited inside anecdote/provenance context (retired-PR narratives like "422 on `.seeds/issues.jsonl` x3", Basis-line history) rather than spec demands; effect: removes a standing false-RED class for every future pass on seeds whose bodies cite the tracker anecdotally (this run passed only because the planner's claim/correction happened to write the file; gate output "1 loop-asset path(s) named, 0 missing"). Adjacent: closed fabro-48f3 explicitly accepted this class as out of scope.

**2. Stop stranding Basis commits: merge instead of squash** — overflow

- overflow: Stop stranding Basis commits: merge instead of squash — set `merge_strategy = "merge"` (or `"rebase"`) in `.fabro/workflows/loop/workflow.toml`; effect: Basis-line shas resolve in the object DB and the planner stops adjudicating every basis via files/worktree workarounds — preflight this run showed 3 of 5 candidates (`fabro-11d3`, `fabro-7f58`, `fabro-b6f9`) with `basis_commit_resolves=false`, and the lane's own `auto_merge=true, merge_strategy="squash"` manufactures more (PR #437's Basis shas strand next). No seed changes the merge strategy (closed fabro-d95c only made the dead sha advisory).

**3. Mirror the per-role command-table split into the loop lane** — overflow

- overflow: Mirror the per-role command-table split into the loop lane — port the `tracker-commands-planner.md` / `tracker-commands-implementer.md` include pattern into `.fabro/workflows/loop/prompts/` (fabro-aeb7's assignee-parameterized fragment is the vehicle); effect: read-only loop roles stop seeing claim/close forms and prompts drop ~1–1.7KB per role per stage — `.fabro/workflows/loop/prompts/project-facts.md` still ships the full claim/close/re-status table to all three roles (measured on develop: reviewer −1692B, implementer −1335B; fabro-52b4 was deliberately develop-scope).

**4. Seed-title correction path** — overflow

- overflow: Seed-title correction path — extend the planner's stale-spec correction arm (`.fabro/workflows/loop/prompts/planner.md` step 3 / `seeds` CLI) with a title-correction form, or have `claim-check.nu` flag stale title terms alongside stale body paths; effect: titles stop contradicting corrected bodies — fabro-52b4's body was corrected pre-claim (`sd`→`seeds`) but its title still reads "…sd-command table … sd show only…", and titles feed `seeds ready` matching, preflight, and briefs every pass.

Linked to already-open overflows (not re-journaled):

- overflow-dup: Make "implementer never runs the battery" mechanical (open in 01M550CAXV85JGT1XBPCDRE5DY.md) — same theme (implementer double-battery run; this run: implementer and tester both ran `just lint-nu && nu .fabro/scripts/prompt-lint.nu`, tester re-ran it green 14s later, ~5KB token echo). Next pass filing the open entry should consider the mechanical-enforcement arm here (`.fabro/workflows/develop/scripts/check-transcript.nu` refusing battery-tier commands when `--by implementer`) as an arm of that seed rather than a sibling.
- overflow-dup: Move the static legend out of planner-preflight routing output (open in 01M536GGFVG16CGN8WPK05WYS7.md) — same finding as review item 9 (legend ~2.3KB of 3.8KB preflight stdout serialized three times per run plus planner preamble; one-line pointer/version-hash instead). No second open entry for this theme.

Not recommended despite being observable: model change or reasoning-effort tuning (cost already $0.068/run, inference-dominated), and the `fabro_run_set_title` retry (2 calls, 1 error; below noise floor).
