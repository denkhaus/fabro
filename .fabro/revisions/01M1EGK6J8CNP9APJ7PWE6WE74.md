# Revision — run 01M1EGK6J8CNP9APJ7PWE6WE74

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M1EGK6J8CNP9APJ7PWE6WE74.md
- seeds filed: fabro-169b Declare lesson_capture in the implementer node's context_allow_keys; fabro-3d2d Add a repo-layout map to the planner prompt or AGENTS.md; fabro-4b57 Emit a loop-work diff in evidence.nu when only churn files changed; fabro-a5de Ban sd prime in develop planner and implementer prompts; fabro-3d04 Make the authorized-exception pattern standing in the implementer prompt; fabro-50f8 Require null-path exercises when editing workflow scripts; fabro-7fba Add evidence to the reviewer's preamble_stages_ignore; fabro-01b9 Path-scope the tester gate for prompt-only seeds

## Findings

- **Declare lesson_capture in the implementer node's context_allow_keys** — filed fabro-169b. One line in `workflow.fabro` (context_allow_keys=implementation_summary,lesson_capture,journal; optionally reviewer preamble_allow_keys) so the fabro-f831 contract's enforced record-or-skip answer lands durably instead of dropping as drift.
- **Add a repo-layout map to the planner prompt or AGENTS.md** — filed fabro-3d2d. 5-line Repo layout section cuts planner@1's ~10 exploratory orientation calls; ~−4.5 min wall and −$0.15 per run. Distinct from sd call-economy seeds (fabro-e4fa/f4cd/1399).
- **Emit a loop-work diff in evidence.nu when only churn files changed** — filed fabro-4b57. When seed_rows is empty but churn_rows is not, emit the per-seed (claim-base) diff under a loop-work diff section so reviewers verify dev-loop seeds from context. Cross-ref fabro-1e9f.
- **Ban sd prime in develop planner and implementer prompts** — filed fabro-a5de. Never run sd prime in this loop; its close/push banner contradicts role rules. Supersedes fabro-3814 (closed: implementer-only conditional skip replaced by the unconditional ban). Complements fabro-cc23.
- **Make the authorized-exception pattern standing in the implementer prompt** — filed fabro-3d04. Platform scope gains: a brief naming a specific platform file authorizes editing exactly that file; planner briefs shed ~a third of their text.
- **Require null-path exercises when editing workflow scripts** — filed fabro-50f8. Exercise null/no-seed/fallback paths via nu -c when editing workflow scripts; nu-check is syntax-only and missed a latent crash (mulch mx-0b966a).
- **Add evidence to the reviewer's preamble_stages_ignore** — filed fabro-7fba. Current capture still arrives via command.output; ~2 KB less preamble per review cycle and stale prior-cycle captures disappear. Complements fabro-699f (not a duplicate: different mechanism).
- **Path-scope the tester gate for prompt-only seeds** — filed fabro-01b9. Route a qualitygate-lite (nu-check + sync) when only `.fabro/`/`.md` files changed; lowest impact of the run.
