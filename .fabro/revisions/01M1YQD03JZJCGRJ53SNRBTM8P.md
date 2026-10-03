# Revision — run 01M1YQD03JZJCGRJ53SNRBTM8P

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M1YQD03JZJCGRJ53SNRBTM8P.md
- seeds filed: fabro-6db3 Classify the engine journal file as expected churn in evidence.nu; fabro-8296 Make hidden-path glob return the fs_hide notice like grep does; fabro-ab38 Sweep open seeds against the merged run diff and mark satisfied siblings superseded
- basis: run 01M1YQD03JZJCGRJ53SNRBTM8P, workflow version 4ff76f32665f954e9a8e2027ec2a4a5b48266707730ae0620fcfd3f79acd4a10, commit 26c7ce34ffeb12df96dc464c485f83e5d1ba0b85
- revised_at_commit: 26c7ce34ffeb12df96dc464c485f83e5d1ba0b85 (ADR-0015: engine drift signal for later judgement)

## Findings

### Classify the engine journal file as expected churn in evidence.nu
- filed: fabro-6db3
- Change: add `.fabro/journal/**` (and `.fabro/blobs/**`) to the known-transient churn section in `.fabro/workflows/develop/scripts/evidence.nu` with an explicit '(engine journal, expected)' label. The stage-journal hook writes after capture, so reviewers see an unexplained `git diff` line and burn verification budget. Distinct from fabro-43ba and fabro-93a7. Effect: every review sees a reconciled churn list; removes a recurring false-deviation risk.

### Make hidden-path glob return the fs_hide notice like grep does
- filed: fabro-8296
- Change: in the `fabro-agent` glob tool, return the same 'hidden from this stage by fs_hide' error grep produces when a glob targets hidden paths, instead of a silent empty result; optionally hoist the fs_hide rule in `.fabro/workflows/develop/prompts/planner.md` to the tool-notes area. The planner burned one LLM round plus 2 tool calls discovering the boundary. Effect: eliminates one wasted inference round per platform-path seed. Not covered by fabro-b7ab or fabro-8d61.

### Sweep open seeds against the merged run diff and mark satisfied siblings superseded
- filed: fabro-ab38
- Change: extend `.fabro/workflows/develop/scripts/closeout.nu` (or the improve workflow) with a post-merge sweep that greps the merged run diff against open seeds targeting the same files and flags already-satisfied ones. Evidence: this run's diff incidentally fixed the planner.md duplicate-'4.' numbering while fabro-890d stays open. Distinct from fabro-aa46, fabro-fbed, and fabro-c74f. Effect: tracker stays truthful after merges; prevents re-picking implemented work.
