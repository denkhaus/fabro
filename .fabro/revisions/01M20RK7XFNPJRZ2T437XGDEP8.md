# Revision — run 01M20RK7XFNPJRZ2T437XGDEP8

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M20RK7XFNPJRZ2T437XGDEP8.md
- seeds filed: fabro-e4c4 (evidence node: deliver capture inline via output_schema), fabro-a08a (planner prompt: visit-1 fresh-cycle line to skip dead context reads)
- basis: run 01M20RK7XFNPJRZ2T437XGDEP8, workflow version af036506627c2c235dc58d52e1ceda475fc4053f219b513e9582f644224dc129, commit 6f23b48525bf1ff7b11d3a4c49ad43681a9ff1fb
- revised_at_commit: 6f23b48525bf1ff7b11d3a4c49ad43681a9ff1fb (ADR-0015: engine drift signal for later judgement)

## Findings

### Develop evidence node: deliver the capture inline via output_schema — filed fabro-e4c4

Give the `evidence` node an `output_schema` (e.g. `@schemas/evidence-capture.schema.json`) and declare it in `context_allow_keys`; swap the reviewer's `preamble_allow_keys` entry from `command.output` to `output.evidence`, mirroring `gatebounce`'s inline `output.gate_known_bug_hits`. In this run the 12.4 KB capture reached the reviewer as a blob ref with a ~300-byte preview, forcing a `read_file` detour plus one extra model round inside the reviewer's 42.0 s inference. Expected effect: one tool call and one model round removed per review; the verification_blocked-on-blob failure mode disappears. Composes with fabro-9ef9 and fabro-c1bb rather than duplicating them.

### Develop planner prompt: visit-1 fresh-cycle line to skip dead context reads — filed fabro-a08a

Add one line to `.fabro/workflows/develop/prompts/planner.md` step 1: visit 1 with no `current_seed_id` in `## Context` means a fresh cycle — go straight to `sd ready`. In this run the planner opened with two `context_read` calls, one erroring on `unknown context key 'current_seed_id'` (events seq 31-34). Expected effect: two fewer tool rounds per fresh planner pass (~$0.02, ~8 s). Orthogonal to fabro-c3b4 and fabro-e4fa.
