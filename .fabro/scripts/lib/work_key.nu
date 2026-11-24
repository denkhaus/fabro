#!/usr/bin/env nu
# work_key (work#263, sprint 99): pure work-item key helpers for the
# goal-vs-grant claim check. The tracker's item keys are `owner/repo#N`
# shapes; a run goal may name the full key or the bare `work#N` form.
# A bare `#N` is prose-ambiguous and is NEVER extracted, and a match
# inside a larger word (network#5) is never extracted either.
# Consumers: planner-preflight.nu (the claim gate);
# planner-preflight-claim-goal-fixtures.nu pins every shape.

# The work-item keys the goal text names, in order of appearance.
export def goal-named-keys [goal: string]: nothing -> list<string> {
    $goal | parse -r '\b(?P<key>(?:[A-Za-z0-9_.\-]+/)?work#\d+)' | get -o key | default []
}

# One canonical key form: a bare `work#N` gains the tracker's owner/repo.
export def normalize-key [key: string]: nothing -> string {
    if ($key =~ '/') { $key } else { $"denkhaus/($key)" }
}

# Pure: does a goal-named key denote the same item as the granted key?
export def keys-match [named: string, granted: string]: nothing -> bool {
    (normalize-key $named) == (normalize-key $granted)
}

# The run goal text from one inspect response record: the goal rides at
# run_spec.settings.run.goal.value (RunGoal {type, value}; an
# InterpString serializes as a plain string). null when the shape
# drifts - the caller degrades open, and the fixture battery pins this
# exact shape so a drift REDs the battery instead of silently
# disabling the check.
export def goal-from-inspect [spec: any]: nothing -> any {
    try { $spec | first | get -o run_spec.settings.run.goal.value } catch { null }
}

# Pure: the inspect argv for the run-goal fetch. The server target is
# EXPLICIT - inside a run sandbox FABRO_SERVER is the channel socket,
# and without the flag the CLI would fall to the local-server default
# (work#263 inspect_fetch decision, sprint 99).
export def inspect-argv [srv: string, run_id: string]: nothing -> list<string> {
    ['inspect' $run_id '--server' $srv '--json']
}
