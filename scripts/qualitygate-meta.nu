#!/usr/bin/env nu
# Platform quality gate (ADR-0004): fabro validate over every workflow,
# nu-check over every workflow script, reviewer-agent at warn threshold.

print "== workflow sync (meta <-> product) =="
nu .fabro/workflows/develop/scripts/sync-check.nu

print "== fabro validate (all workflows) =="
let graphs = (glob .fabro/workflows/*/workflow.fabro)
if ($graphs | is-empty) { print "no workflows found"; exit 1 }
for g in $graphs {
    let r = (fabro validate $g | complete)
    if $r.exit_code != 0 { print $r.stdout; print $r.stderr; exit 1 }
}
print $"validated ($graphs | length) workflows"

print "== nu-check (all workflow scripts) =="
let scripts = (glob .fabro/workflows/*/scripts/*.nu)
for s in $scripts {
    let r = (nu -c $"nu-check ($s)" | complete)
    if ($r.exit_code != 0) or (not ($r.stdout | str trim | str ends-with "true")) {
        print $"script failed nu-check: ($s)"
        print $r.stdout
        exit 1
    }
}
print $"syntax-clean ($scripts | length) scripts"

print "== reviewer-agent (develop, min_severity=warn) =="
# The platform operates the develop pipeline; strict review scopes to it.
# Validate and nu-check above still cover every workflow.
let review = (uv run --python 3.12 --no-project python -c "
import sys
sys.path.insert(0, '.prime/agent/skills/reviewer-agent/src')
import reviewer_agent
import json
findings = json.loads(reviewer_agent.run(
    root='.', workflow='develop', format='json',
    allowed_tools=('just','ml','sd','nu','nushell'),
    min_severity='info', report_dir=None,
))
bad = [f for f in findings if f['severity'] in ('error','warn')]
print(json.dumps([{'rule': f['rule'], 'severity': f['severity'], 'path': f['path']} for f in bad], indent=1))
sys.exit(1 if bad else 0)
" | complete)
if ($review.exit_code != 0) {
    print $review.stdout
    print $review.stderr
    exit 1
}
print "reviewer-agent: 0 errors, 0 warnings"

print "== platform qualitygate passed =="
