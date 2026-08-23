# The workflow is the infrastructure — change it as one unit

Everything inside `.fabro/workflows/develop/` is infrastructure and changes
together: the graph (`workflow.fabro`), every prompt, every script
(`evidence.nu`, `refiner.nu`, `bootstrap.nu`, `qualitygate.nu`), the config
surface (`workflow.toml`), and every setting they reference
(`preamble_budget_kb`, fidelity, `preamble_stages_ignore`, model settings in
`project.toml`). A change to ANY layer invalidates the contracts of the
others: when engine semantics change (value demotion, preamble budgets, tool
availability), every prompt asserting the old behavior, every script comment
documenting the old mechanism, and every seed encoding the old acceptance
must be updated in the SAME change, before the next run. The observed cost of
violating this (run 01M0QJ5FNE9): the engine delivered a blob ref exactly as
designed, the reviewer prompt still asserted tool-less single-call review,
and a full implement→gate→evidence→review cycle burned on the mismatch.
AGENTS.md deliberately stays minimal — it is agent-facing context, not the
home of platform rules; process rules live here in the ADRs.
