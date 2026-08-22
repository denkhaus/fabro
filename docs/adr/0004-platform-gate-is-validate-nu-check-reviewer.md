# The platform quality gate is workflow lint, not product tests

`just qualitygate` on the meta branch runs, in order: `fabro validate` over
every workflow graph, `nu-check` over every workflow script, and the
reviewer-agent with `min_severity=warn` (zero errors, zero warnings
required). Meta implementers are subject to the same loop discipline as
product implementers; they only answer to a different gate.
