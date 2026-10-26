# The platform quality gate is workflow lint, not product tests

`just qualitygate` on the meta branch runs, in order: `fabro validate` over
every workflow graph, `nu-check` over every workflow script, and the
reviewer-agent with `min_severity=warn` scoped to `develop` (zero errors,
zero warnings required) — validate/nu-check cover every workflow, strict
review covers the pipeline the platform actively operates. Legacy
playground workflows therefore stay executable but are not held to the
operating standard. Meta implementers are subject to the same loop discipline as
product implementers; they only answer to a different gate.
