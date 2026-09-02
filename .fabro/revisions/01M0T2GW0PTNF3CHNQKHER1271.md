# Revision — run 01M0T2GW0PTNF3CHNQKHER1271

- status reviewed: failed
- review: .fabro/reviews/develop/01M0T2GW0PTNF3CHNQKHER1271.md
- seeds filed: fabro-3c9d Prioritize source files and enlarge diff budget in evidence capture
- seeds filed: fabro-7773 Lint develop seeds for spec contradictions before tracker entry
- seeds filed: fabro-45bf Compute pipeline progress from unique completed nodes
- seeds filed: fabro-46f3 Honor configured PR model and retry before deterministic fallback

## Findings

### Stop terminal-failing develop runs on PR publish errors
Duplicate of fabro-6a77 (and fabro-696c): downgrade or disable the `pull_request` publish step in `.fabro/workflows/develop/workflow.toml` so gate+review-passing runs are not archived as `failed(publish_failed)` on a 403. Not re-filed.

### Prioritize source files and enlarge diff budget in evidence capture
Filed as fabro-3c9d. Sort seed files code-first and raise `OUTPUT_BUDGET` in `.fabro/workflows/develop/scripts/evidence.nu`. Effect: reviewer verifies from the capture directly.

### Make the develop preamble budget coherent
Duplicate of fabro-83f9 (raise preamble budget 12→~24KB) with overlap against fabro-699f (stop re-rendering past cycles). Not re-filed.

### Scope skill discovery to the workflow skills directory
Duplicate of fabro-2c81. Not re-filed.

### Forbid `sd prime` in planner and implementer prompts
Duplicate of fabro-3814 (implementer scope); the planner extension can be folded into that seed. Not re-filed.

### Lint develop seeds for spec contradictions before tracker entry
Filed as fabro-7773. Contradiction check at seed authoring time. Effect: saves planner deliberation time and reviewer ping-pong on ambiguous seeds.

### Compute pipeline progress from unique completed nodes
Filed as fabro-45bf. Unique completed non-meta nodes over total in the progress projection. Effect: honest mid-loop progress numbers.

### Honor configured PR model and retry before deterministic fallback
Filed as fabro-46f3. Honor `pull_request.model` and retry without strict JSON before fallback. Effect: meaningful PR titles/bodies once publishing works.
