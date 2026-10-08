#!/usr/bin/env nu
# One battery-runner surface (fabro-eae5, arch-gate sprint 24 C3): the
# loop-asset battery registry lives HERE and only here. Before this
# script the path lists sat as literals in scripts/qualitygate.nu
# (check-loop-assets) and the loop lane's tester gate
# (.fabro/workflows/loop/scripts/loop-gate.nu) ran none of them — the
# only defense was prose telling a session to run batteries by hand.
#
# Both callers route through this file:
#   product gate:  nu .fabro/scripts/battery-runner.nu all
#                  (replaces check-loop-assets' inline tier)
#   loop gate:     nu .fabro/scripts/battery-runner.nu all
#                  (lint + batteries tier; a red battery REDs the loop
#                  lane's tester gate)
#
# Scope surface (target contract):
#   lint      — lint-nu (every repo nu script) + prompt-lint
#   batteries — the registered smokes and fixture batteries below
#   all       — lint then batteries
#
# Exit 0 = green; first red stops and exits 1 (qualitygate style).
# Paths are repo-root-relative: both callers run from the repo root.

# Registered smokes: pure-logic regression scripts (no live server, no
# cargo). Discovery is explicit and minimal — name each battery; do NOT
# blanket-run every .fabro/scripts/*.nu (stage-journal.nu and
# friction-score.nu are tools, not batteries).
const SMOKES = [
    '.fabro/workflows/develop/scripts/evidence-smoke.nu'
    # Claim-gate path battery (fabro-4c81): the pure claim-body-verdict
    # core over canned seeds-show records — blocking classes, creation-
    # intent windows, advisory classes, fail-open contract.
    '.fabro/workflows/develop/scripts/claim-check-smoke.nu'
    # Graph-contract pin (fabro-83df/fabro-92e2, incident 2026-09-19):
    # the develop graph must keep its deterministic-exit contract —
    # planner ungated, preflight report-only, guard exits intact.
    '.fabro/workflows/develop/scripts/graph-contract-smoke.nu'
    # Loop-lane graph-contract pin (fabro-70b5): lane wiring flags,
    # the implementer envelope pin, the red bounce — the meta lane's
    # load-bearing contracts, same tier as the develop pin.
    '.fabro/workflows/loop/scripts/graph-contract-smoke.nu'
    # Revisor graph-contract pin (fabro-2357): the meta lane stays a
    # MANUAL fire lane, so the toolchain-placement guard is an operator
    # path there too — first stage after start, POSIX sh, no bypass
    # edge. The revisor had no graph-contract smoke before this.
    '.fabro/workflows/revisor/scripts/graph-contract-smoke.nu'
    # tracker-guard pure decision logic (fabro-0da8): guard-decision /
    # sd-issue-count over canned complete-style records — both-empty
    # route, open/in_progress arms, and the sd-failure fail-open
    # contract. Found UNREGISTERED by the fabro-8b38 registration
    # sweep (the ac84 silent-de-gate class): the script existed, ran
    # green, and nothing executed it.
    '.fabro/workflows/develop/scripts/tracker-guard-smoke.nu'
    # Salvage-sweep analysis battery (fabro-f312, user directive
    # 2026-09-30): the dump analysis is the sweep's decision core, so
    # the battery pins the five verdict-relevant shapes — failed with
    # real work, journal-only bookkeeping, the green-lie, a diff-less
    # run, and a clean green run — with no live server in reach.
    '.fabro/scripts/salvage-sweep-smoke.nu'
    # closeout pure-decision logic (fabro-5af4/591a era): reviewer
    # journal, deferred-action and exemption-arm sweep. Same finding —
    # unregistered until fabro-8b38.
    '.fabro/workflows/develop/scripts/closeout-smoke.nu'
    # close-claim battery (fabro-2a3b): the close-claim-check core over
    # the incident fixture pair — a subject claiming a close the tracker
    # never recorded is a finding; remainder/residual/file phrasings
    # stay quiet. The LIVE check runs in the session's line watch.
    '.fabro/scripts/close-claim-check-smoke.nu'
    # prompt-lint marker battery (fabro-cb5c): the synthetic-fixture-id
    # marker must silence ONLY its own file — the check keeps teeth on
    # unmarked files and every other prompt-lint check stays live on a
    # marked one. Runs the real lint against temp roots.
    '.fabro/scripts/prompt-lint-fixtures.nu'
    # release-sha battery (fabro-06da): release tags must name the
    # PUSHABLE line tip — only the allow-listed files may derive a short
    # sha, the two release scripts must call the policy site, and the
    # scanner proves it has teeth on a planted fixture.
    '.fabro/scripts/release-sha-fixtures.nu'
]

# Registered checked-in fixture batteries (seed fabro-ac84): the gate
# must EXECUTE the fixture scripts, not just parse them.
const BATTERIES = [
    '.fabro/scripts/dup-run-check-fixtures.nu'
    # planner-preflight anchor battery (fabro-83df report-only
    # end-to-end case included; 0.5s measured 2026-09-19)
    '.fabro/scripts/planner-preflight-anchor-fixtures.nu'
    # revisor overflow-ledger battery (fabro-552a): fixture revision
    # file with open + consumed overflows drives `open`/`consume`
    # selection deterministically, plus the live real-tree invariant
    # for the memoize entry in revision 01M2X8458MDWMRVBRVEMDX9W4J.
    '.fabro/workflows/revisor/scripts/overflow-ledger-fixtures.nu'
    # run-scope fixtures (fabro-70b5 part D): the run-scope
    # classification RED both ways, both lanes, plus the git base
    # derivation — this tier proves the meta lane's diff boundary.
    '.fabro/scripts/run-scope-fixtures.nu'
    # rust-style-guide skill parity (fabro-6538): .fabro/skills/
    # rust-style-guide (canonical — run-facing, the reviewer
    # contract's source) and .agents/skills/rust-style-guide (the
    # local-session mirror) must stay byte-identical; the battery
    # hash-compares both trees (run-images.nu's sha256 pattern).
    '.fabro/scripts/skill-parity-fixtures.nu'
    # touchpoints parity (fabro-de32, arch-gate sprint 9): every
    # tracked fork-only pin file must be named in the touchpoints
    # registry and every literal row path must exist — the two-pin
    # rule machine-checked in both directions (was reviewer-prompt
    # only; live gaps found at filing: fork_exec_guard x2,
    # fork_structured).
    '.fabro/scripts/touchpoints-parity-fixtures.nu'
]

def run-list [kind: string, paths: list<string>]: nothing -> bool {
    for path in $paths {
        let res = (do { ^nu $path } | complete)
        if $res.exit_code != 0 {
            print -e $"battery-runner: ($kind) FAILED: ($path)"
            print ($res.stdout | str trim -r -c "\n" | lines | last 20)
            print -e ($res.stderr | str trim -r -c "\n")
            return false
        }
    }
    true
}

def "main lint" []: nothing -> nothing {
    print '== battery-runner lint: lint-nu (every repo nu script) =='
    let lint = (do { ^nu scripts/lint-nu.nu } | complete)
    print $lint.stdout
    if ($lint.exit_code != 0) {
        print -e $lint.stderr
        exit 1
    }
    print '== battery-runner lint: prompt-lint (literal hygiene) =='
    let plint = (do { ^nu .fabro/scripts/prompt-lint.nu } | complete)
    print $plint.stdout
    if ($plint.exit_code != 0) {
        print -e $plint.stderr
        exit 1
    }
    print 'lint green'
}

def "main batteries" []: nothing -> nothing {
    print $"== battery-runner batteries: ($SMOKES | length) smoke\(s\) + ($BATTERIES | length) fixture batter\(y/ies\) =="
    if not (run-list 'smoke' $SMOKES) { exit 1 }
    if not (run-list 'fixture battery' $BATTERIES) { exit 1 }
    print 'batteries green'
}

def main [scope: string = "all"]: nothing -> nothing {
    if not ($scope in ["lint" "batteries" "all"]) {
        print -e $"battery-runner: unknown scope '($scope)' \(lint|batteries|all\)"
        exit 2
    }
    if ($scope in ["lint" "all"]) { main lint }
    if ($scope in ["batteries" "all"]) { main batteries }
    print 'BATTERY RUNNER GREEN'
}
