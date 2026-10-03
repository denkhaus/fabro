# Revision — run 01M23B02H3ZNND8AT2D3Y2S4SG

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M23B02H3ZNND8AT2D3Y2S4SG.md
- seeds filed:
  - fabro-7ac9 — Engine: per-seed fs_hide exception when the claimed brief targets only hidden paths (capability-affecting, needs-user)
  - fabro-8cef — Evidence capture: emit the reviewer-facing blob as plain-text multi-line, not a JSON-escaped single line
  - fabro-9967 — Planner prompt: cap sd ready to a top-N view and pass created_since to fabro_runs_list
  - fabro-607a — Keep concrete literals at PROJECT_FACTS indirection sites
  - fabro-b769 — Add a long-stage heartbeat notification for run visibility
- basis: run 01M23B02H3ZNND8AT2D3Y2S4SG, workflow version 3bd915204178a8e5fd6796670d1bbfae4248a4b01f5d0fb7201e53b394cfdda8, commit db6b8ca5b4f7c0fc7583e1d28df1fa9ba295958e
- revised_at_commit: db6b8ca5b4f7c0fc7583e1d28df1fa9ba295958e (ADR-0015: engine drift signal for later judgement)

## Findings

### Engine: per-seed fs_hide exception when the claimed brief targets only hidden paths
- filed: fabro-7ac9 (labels: needs-user,revision — capability-affecting per ADR-0019: changes agent file-tool permissions on hidden paths; implementation awaits explicit user approval)
- Change: fs_hide resolution in `lib/components/fabro-workflow` allows file tools for paths that the claimed seed brief targets when all targets fall inside the fs_hide list. Not covered by open fabro-d02d (denial message) or fabro-8296 (glob notice) — thematic overlap only, cross-referenced.
- Expected effect: eliminates the discovery turn and heredoc-edit mode for workflow-asset-targeting seeds (implementer was 86% of run cost, 84% of wall).

### Evidence capture: emit the reviewer-facing blob as plain-text multi-line
- filed: fabro-8cef
- Change: `scripts/evidence.nu` writes the capture blob as plain-text multi-line instead of a single-line JSON-escaped string. Budget-raise alternative dropped as duplicate of open fabro-35ab / fabro-3c9d.
- Expected effect: one read_file instead of three substring pages per review; removes misparse risk, ~60–90s reviewer latency on asset-heavy seeds.

### Planner prompt: cap sd ready and recency-filter fabro_runs_list
- filed: fabro-9967
- Change: command table in `.fabro/workflows/develop/prompts/planner.md` uses `--limit 10` for `sd ready` and passes a recency filter to `fabro_runs_list`.
- Expected effect: ~35KB less planner context in early turns, faster first output, less tie-break distraction.

### Keep concrete literals at PROJECT_FACTS indirection sites
- filed: fabro-607a
- Change: planner.md step 4 and `prompts/project-facts.md` use sites carry the concrete example ("the seed id prefix (e.g. `fabro-`)") alongside the table.
- Expected effect: removes a misparse class from the double-pick guard mechanism; near-zero cost. Related to closed fabro-4814 (thematic, not supersession).

### Add a long-stage heartbeat notification for run visibility
- filed: fabro-b769
- Change: notifications block in `.fabro/workflows/develop/workflow.toml` adds a stage-level event or "stage exceeds N minutes" heartbeat alongside terminal events.
- Expected effect: mid-run visibility into single-stage cost concentration while intervention is still possible.
