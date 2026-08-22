#!/usr/bin/env nu
# Platform quality gate (ADR-0004): workflow sync, fabro validate over
# every workflow, nu-check over every workflow script, reviewer-agent at
# warn threshold scoped to develop (the pipeline the platform operates).
# Exit 0 = green. Each check prints its own section and returns false
# (with details) on failure; main stops at the first red check.

# ---------------------------------------------------------------------------
# checks — nothing -> bool (false = red, details already printed)
# ---------------------------------------------------------------------------

def check-sync []: nothing -> bool {
    print "== workflow sync (meta <-> product) =="
    let res = (do { nu .fabro/workflows/develop/scripts/sync-check.nu } | complete)
    print ($res.stdout | str trim -r -c "\n")
    if $res.exit_code != 0 {
        print ($res.stderr | str trim -r -c "\n")
        return false
    }
    true
}

def check-validate []: nothing -> bool {
    print "== fabro validate (all workflows) =="
    let graphs = (glob .fabro/workflows/*/workflow.fabro)
    if ($graphs | is-empty) {
        print "no workflows found"
        return false
    }
    let broken = ($graphs | each {|g|
        let res = (do { fabro validate $g } | complete)
        if $res.exit_code != 0 { {graph: $g, out: $"($res.stdout)\n($res.stderr)"} } else { null }
    } | compact)
    if ($broken | is-not-empty) {
        $broken | each {|b| print $"FAILED: ($b.graph)\n($b.out)" }
        return false
    }
    print $"validated ($graphs | length) workflows"
    true
}

def check-scripts []: nothing -> bool {
    print "== nu-check (all nu scripts) =="
    # ADR-0006: every nu script in this world's tree — workflow scripts
    # and root scripts alike must parse.
    let scripts = [(glob .fabro/workflows/*/scripts/*.nu) (glob scripts/*.nu)] | flatten
    if ($scripts | is-empty) {
        print "no workflow scripts found"
        return false
    }
    # nu-check is an internal command: try/catch yields a plain bool
    # (false covers parse failures and unreadable files alike).
    let broken = ($scripts | each {|s|
        if (try { nu-check $s } catch { false }) { null } else { {script: $s} }
    } | compact)
    if ($broken | is-not-empty) {
        $broken | each {|b| print $"script failed nu-check: ($b.script)" }
        return false
    }
    print $"syntax-clean ($scripts | length) scripts"
    true
}

def check-review []: nothing -> bool {
    print "== reviewer-agent (develop, min_severity=warn) =="
    # The platform operates the develop pipeline; strict review scopes to it.
    # Validate and nu-check above still cover every workflow.
    let res = (do {
        uv run --python 3.12 --no-project python -c "
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
"
    } | complete)
    if $res.exit_code != 0 {
        print $res.stdout
        print $res.stderr
        return false
    }
    print "reviewer-agent: 0 errors, 0 warnings"
    true
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

def main []: nothing -> nothing {
    if not (check-sync) { exit 1 }
    if not (check-validate) { exit 1 }
    if not (check-scripts) { exit 1 }
    if not (check-review) { exit 1 }
    print "== platform qualitygate passed =="
}
