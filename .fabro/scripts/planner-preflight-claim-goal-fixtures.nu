#!/usr/bin/env nu
# prompt-lint: synthetic-seed-ids
# Fixture battery for the planner-preflight goal-vs-grant claim check
# (work#263, sprint 99): unit-drives the pure key helpers (goal-key
# extraction, normalization, match, inspect argv) and drives
# planner-preflight.nu END TO END in --work-item mode over stdin
# packets through the --goal fixture seam. A goal naming a different
# item key REDS the claim contract (exit 1 + the full teaching
# refusal); a matching key - bare or full form - passes with the
# goal_check record in the envelope; a generic key-less goal passes
# un-compared; the no-run-id path skips open. No live server, no
# tracker, no git: the e2e runs in a scratch dir where dup-run-check
# degrades (fail-open) by design.
#
# Usage: nu .fabro/scripts/planner-preflight-claim-goal-fixtures.nu
# (exit 0 = all pass)

const FIXTURES_DIR = (path self | path dirname)
source ($FIXTURES_DIR | path join 'lib' 'work_key.nu')

const PREFLIGHT = ($FIXTURES_DIR | path join '..' 'workflows' 'develop' 'scripts' 'planner-preflight.nu')

def expect [case: string, got, want] {
    if $got == $want {
        print $"PASS ($case)"
    } else {
        print -e $"FAIL ($case): got ($got | to json --raw), want ($want | to json --raw)"
        exit 1
    }
}

def expect-contains [case: string, haystack: string, needle: string] {
    if ($haystack | str contains $needle) {
        print $"PASS ($case)"
    } else {
        print -e $"FAIL ($case): needle not found in output"
        print -e $"  needle: ($needle)"
        print -e $"  output head: ($haystack | str substring 0..400)"
        exit 1
    }
}

def run-preflight [goal: string, item_key: string] {
    let pkt = ({itemKey: $item_key, title: "fixture item", body: "fixture body without anchors"} | to json --raw)
    let args = (if ($goal | is-empty) { ['--work-item'] } else { ['--work-item' '--goal' $goal] })
    with-env {FABRO_RUN_ID: '', FABRO_SERVER: ''} {
        $pkt | run-external 'nu' $PREFLIGHT ...$args
    } | complete
}

def main [] {
    let home_dir = $env.PWD
    let scratch = (mktemp -d -t claim-goal-fixtures.XXXXXX)
    cd $scratch

    try {
    # Unit: goal-key extraction (full key, bare repo key, generic goal,
    # bare #N prose refs never extracted, word-boundary guard).
    expect 'u1: full key extracted' (goal-named-keys 'Work denkhaus/work#1434: title here') ['denkhaus/work#1434']
    expect 'u2: bare repo key extracted' (goal-named-keys 'claim work#999 now') ['work#999']
    expect 'u3: generic goal has no key' (goal-named-keys 'Develop the project one seed per run: claim the next open seed, implement, gate, review, exit') []
    expect 'u4: bare #N never extracted' (goal-named-keys 'fixes #1434 in prose') []
    expect 'u5: no key inside words' (goal-named-keys 'Network#5 roadmap work') []
    expect 'u6: two keys in order' (goal-named-keys 'Run denkhaus/work#1234 then work#1234') ['denkhaus/work#1234', 'work#1234']

    # Unit: normalization + match.
    expect 'u7: normalize bare' (normalize-key 'work#1434') 'denkhaus/work#1434'
    expect 'u8: normalize full stays' (normalize-key 'denkhaus/work#1434') 'denkhaus/work#1434'
    expect 'u9: match identical' (keys-match 'denkhaus/work#1434' 'denkhaus/work#1434') true
    expect 'u10: match bare vs full' (keys-match 'work#1434' 'denkhaus/work#1434') true
    expect 'u11: different id mismatches' (keys-match 'denkhaus/work#1434' 'denkhaus/work#999') false
    expect 'u12: different repo mismatches' (keys-match 'other/work#1434' 'denkhaus/work#1434') false

    # Unit: the inspect argv shape (channel-router transport, work#263
    # inspect_fetch decision - explicit --server, never the default).
    expect 'u13: inspect argv shape' (inspect-argv 'http://fabro' '01ABC') ['inspect' '01ABC' '--server' 'http://fabro' '--json']

    # E2E: mismatch REDS the claim contract with the full refusal.
    let r1 = (run-preflight 'Work denkhaus/work#999: other topic' 'denkhaus/work#1434')
    expect 'e1: mismatch exit' $r1.exit_code 1
    expect-contains 'e1: refusal names both keys' $r1.stderr "claim preflight: goal names 'denkhaus/work#999' but the granted claim is 'denkhaus/work#1434'"
    expect-contains 'e1: refusal carries the fix' $r1.stderr 'assign the named item or drop the name from the goal'

    # E2E: matching key passes, bare form equals full form.
    let r2 = (run-preflight 'Work denkhaus/work#1434: the right one' 'denkhaus/work#1434')
    expect 'e2: match exit' $r2.exit_code 0
    let env2 = ($r2.stdout | from json | get context_updates | get "output.preflight" | get goal_check)
    expect 'e2: status' $env2.status 'pass'
    expect 'e2: named key' $env2.named_key 'denkhaus/work#1434'
    expect 'e2: granted key' $env2.granted_key 'denkhaus/work#1434'
    let r2b = (run-preflight 'Work work#1434: bare form of the same item' 'denkhaus/work#1434')
    expect 'e2b: bare-form match exit' $r2b.exit_code 0
    let env2b = ($r2b.stdout | from json | get context_updates | get "output.preflight" | get goal_check)
    expect 'e2b: bare-form status' $env2b.status 'pass'

    # E2E: a generic key-less goal passes un-compared.
    let r3 = (run-preflight 'Develop the project one seed per run: claim the next open seed, implement, gate, review, exit' 'denkhaus/work#1434')
    expect 'e3: generic exit' $r3.exit_code 0
    let env3 = ($r3.stdout | from json | get context_updates | get "output.preflight" | get goal_check)
    expect 'e3: status' $env3.status 'pass'
    expect 'e3: named key null' $env3.named_key null

    # E2E: no goal flag and no run id - skipped open.
    let r4 = (run-preflight '' 'denkhaus/work#1434')
    expect 'e4: skip exit' $r4.exit_code 0
    let env4 = ($r4.stdout | from json | get context_updates | get "output.preflight" | get goal_check)
    expect 'e4: status' $env4.status 'skipped'

    # Unit: the live inspect response shape is pinned - a shape drift
    # REDs here instead of silently disabling the check in production
    # (the check is fail-open, so a drifted parse would read as
    # 'goal check skipped' forever).
    let good = [{run_spec: {settings: {run: {goal: {type: inline, value: 'Run work#1434: topic'}}}}}]
    expect 'u14: goal from good inspect shape' (goal-from-inspect $good) 'Run work#1434: topic'
    expect 'u15: missing run_spec degrades null' (goal-from-inspect [{}]) null
    expect 'u16: non-string goal degrades null' (goal-from-inspect [{run_spec: {settings: {run: {goal: {type: file, value: null}}}}}]) null

    print 'claim-goal fixtures: all green'
    } finally {
        cd $home_dir
        rm -rf $scratch
    }
}
