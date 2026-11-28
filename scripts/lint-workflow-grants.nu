#!/usr/bin/env nu
# lint-workflow-grants — the project's allow-list of engine-door grants
# (ADR-0052, denkhaus/work#1716).
#
# The engine knows no workflow by name. What a workflow's runs may do at
# the engine's doors is the workflow's own declaration: the
# `[run.grants]` block of its workflow.toml (verdict_fields,
# observation_triage, work_item_close, merge, resolve_push). A
# declaration is a capability, so this script is the project's review
# of it — the grant twin of lint-tool-grants.
#
# Shape: for every .fabro/workflows/*/workflow.toml, read [run.grants]
# and compare it with the ALLOWED table below. Findings:
#   HARD  — a workflow declares a grant the table does not admit.
#   HARD  — two workflows declare the same verdict field: each field has
#           exactly ONE declaring workflow per project (ADR-0039
#           decision 6). The engine cannot see this — it reads one
#           definition at a time — so the project checks it here.
#   HARD  — a table row names a workflow directory that does not exist
#           (a stale row).
#   ok    — the declaration equals its row. A narrower declaration than
#           the row is ok too: the row is a ceiling, not a demand.
#
# To admit a new grant: decide it, then extend the row with the why in
# the commit. Never widen a row to silence this lint without a decision.
#
# Exit code: any HARD finding -> 1; clean -> 0; no workflow
# definitions found -> 2 (broken scope).

# The allow-list: one row per workflow that declares anything. Each row
# is the ceiling of what that workflow may declare.
const ALLOWED = [
    {
        workflow: 'merger'
        verdict_fields: ['closable']
        observation_triage: false
        work_item_close: false
        merge: true
        resolve_push: true
    }
    {
        workflow: 'adjudicator'
        verdict_fields: ['merge']
        observation_triage: false
        work_item_close: true
        merge: false
        resolve_push: false
    }
    {
        workflow: 'triager'
        verdict_fields: []
        observation_triage: true
        work_item_close: false
        merge: false
        resolve_push: false
    }
]

const BOOL_KEYS = [observation_triage, work_item_close, merge, resolve_push]

def main []: nothing -> nothing {
    let files = (glob '.fabro/workflows/*/workflow.toml')
    if ($files | is-empty) {
        print 'lint-workflow-grants: no workflow definitions under .fabro/workflows'
        exit 2
    }

    mut findings: list<string> = []
    mut declared_by: record = {}
    for file in $files {
        let workflow = ($file | path dirname | path basename)
        let grants = (open --raw $file | from toml | get -o run? | get -o grants? | default {})
        let row = ($ALLOWED | where workflow == $workflow | first)
        let fields = ($grants | get -o verdict_fields? | default [] | each { into string } | sort)
        for field in $fields {
            let before = ($declared_by | get -o $field | default [])
            $declared_by = ($declared_by | upsert $field ($before | append $workflow))
        }
        if ($row | is-empty) {
            if (not ($grants | is-empty)) {
                $findings = ($findings | append $"'($workflow)': declares grants but has no row in ALLOWED — decide the grant, then add the row: ($grants | to json)'")
            }
            continue
        }
        let allowed_fields = ($row.verdict_fields | each { into string } | sort)
        for field in $fields {
            if ($field not-in ($allowed_fields | each { into string })) {
                $findings = ($findings | append $"'($workflow)': declares verdict field '($field)' the allow-list does not admit; admitted: ($allowed_fields | str join ', ')'")
            }
        }
        for key in $BOOL_KEYS {
            let declared = ($grants | get -o $key | default false | into bool)
            let allowed = ($row | get $key | into bool)
            if $declared and not $allowed {
                $findings = ($findings | append $"'($workflow)': declares '($key) = true' the allow-list does not admit'")
            }
        }
        print $"'($workflow)': ok — ($grants | to json)"
    }

    for field in ($declared_by | columns) {
        let workflows = ($declared_by | get $field)
        if ($workflows | length) > 1 {
            $findings = ($findings | append $"'($field)': declared by ($workflows | length) workflows — ($workflows | str join ', '); exactly one workflow may declare a verdict field, ADR-0039 decision 6'")
        }
    }

    let known = ($files | each { path dirname | path basename })
    for row in $ALLOWED {
        if ($row.workflow not-in $known) {
            $findings = ($findings | append $"'($row.workflow)': a row in ALLOWED names no workflow directory — remove the stale row'")
        }
    }

    if ($findings | is-empty) {
        print $'lint-workflow-grants: clean — ($files | length) workflow definitions checked'
        return
    }
    for finding in $findings {
        print $'HARD ($finding)'
    }
    print $'lint-workflow-grants: HARD findings: ($findings | length)'
    exit 1
}
