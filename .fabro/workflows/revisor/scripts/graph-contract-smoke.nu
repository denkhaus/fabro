#!/usr/bin/env nu
# Graph-contract smoke (fabro-2357): pins the revisor graph's
# toolchain-placement contract. The revisor lane stays a MANUAL fire lane,
# so a flag-less `fabro run revisor` is a real operator path — without the
# guard it burns a pass before the first stage that needs the toolchain
# (the agent stages shell out to nu/seeds), on the server's default sandbox
# image. Pure file assertions, no
# seeds, no git — the develop/loop graph-contract-smoke pattern (wired into
# scripts/qualitygate.nu's loop-asset tier).
#
#   nu .fabro/workflows/revisor/scripts/graph-contract-smoke.nu
#
# Contract:
#   1. Toolchain-placement guard (fabro-2357, the loop-lane pattern of
#      fabro-3ab2): start routes through env_guard before select, the node
#      runs the shared guard through POSIX sh with the revisor lane flag,
#      and start has NO second outgoing edge. A dropped or bypassed guard
#      silently returns the cryptic "nu: command not found" death.

const GRAPH = ('.fabro/workflows/revisor/workflow.fabro' | path expand)

def fail [what: string]: nothing -> nothing {
    print -e $"revisor graph-contract-smoke: FAIL — ($what)"
    exit 1
}

def main [] {
    if not ($GRAPH | path exists) { fail $"graph not found: ($GRAPH)" }
    let lines = (open --raw $GRAPH | lines)

    # Assertions read COMMENT-STRIPPED lines (the loop smoke's precedent):
    # the rationale comments name the contract, and only real code may
    # satisfy it.
    let code = ($lines | each {|l| ($l | split row '//' | first)})
    if not ($code | any {|l| ($l | str contains 'start -> env_guard')}) {
        fail 'start -> env_guard edge missing — a misplaced manual fire would reach the agent stages without the toolchain check'
    }
    if not ($code | any {|l| ($l | str contains 'env_guard -> select')}) {
        fail 'env_guard -> select edge missing — the placement guard must precede every stage that needs the toolchain'
    }
    if not ($code | any {|l| ($l | str contains 'toolchain-guard.sh revisor') and ($l | str starts-with '        script="sh ')}) {
        fail 'env_guard script line must run the shared guard via POSIX sh with the revisor lane flag — nu cannot guard a sandbox without nu'
    }
    let start_edges = ($code | where {|l| ($l | str trim | str starts-with 'start ->')})
    if ($start_edges | length) != 1 {
        fail $'start must have exactly ONE outgoing edge, the placement guard; found ($start_edges | length) — a second edge would bypass it'
    }

    print 'revisor graph-contract-smoke: OK — placement guard first, no bypass edge'
}
