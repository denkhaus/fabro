#!/usr/bin/env nu
# Smoke test for claim-check.nu's PURE path-gate core (fabro-4c81):
# claim-body-verdict over canned `complete`-style seeds-show records
# against a scratch worktree — blocking classes (repo-rooted missing
# citation, line-anchored missing citation), non-blocking classes
# (creation-intent window, unrooted bare token, out-of-range anchor,
# URL :port shapes), resolution fallbacks (workflow-relative anchored
# AND bare citations across non-develop workflow dirs), and the
# fail-open contract (seeds exit, non-JSON, success:false, missing
# issue row). The id-contract arm of main (stdin shape) stays a manual
# invocation check — sourcing does not run main (closeout-smoke
# pattern). The `source` const resolves against THIS file's directory,
# so the script runs from any cwd:
#   nu .fabro/workflows/develop/scripts/claim-check-smoke.nu

const CLAIM_CHECK = "claim-check.nu"
const CLAIM_SMOKE_SCRIPT = "claim-check.nu"  # dry-run target; nu resolves consts against this file's dir at use time
source $CLAIM_CHECK

def fail [what: string]: nothing -> nothing {
    print -e $"claim-check-smoke: FAIL — ($what)"
    exit 1
}

def expect-ok [case: string, v: record] {
    if $v.outcome != "succeeded" { fail $"($case): expected succeeded, got ($v | to json --raw)" }
    if $v.degraded { fail $"($case): expected non-degraded" }
    if ($v.blocking | is-not-empty) { fail $"($case): expected no blocking flags, got ($v.blocking | to json --raw)" }
    print $"PASS \($case\)"
}

def expect-degraded [case: string, v: record] {
    if $v.outcome != "succeeded" { fail $"($case): degraded must still succeed, got ($v | to json --raw)" }
    if not $v.degraded { fail $"($case): expected degraded" }
    print $"PASS \($case\)"
}

def show-with [desc: string]: nothing -> record {
    {"exit_code": 0, "stdout": ('{"success":true,"issue":{"id":"fabro-x1","description":' + ($desc | to json --raw) + '}}')}
}

# Top-level statements, no def main: the sourced claim-check.nu already
# defines main (duplicate_command_def otherwise; closeout-smoke pattern).
let home_dir = $env.PWD
let script_dir = ($env.CURRENT_FILE | path dirname)
let scratch = (mktemp -d -t claim-check-smoke.XXXXXX)

try {
    cd $scratch
    mkdir pkg
    seq 1 200 | each {|i| $"line ($i) content"} | save -f pkg/a.rs
    # non-develop workflow dir: proves the loop lane's relative
    # citations resolve too (workflow-dirs fallback)
    mkdir .fabro/workflows/loop/prompts
    ["# planner" "one" "two"] | save -f .fabro/workflows/loop/prompts/planner.md
    mkdir .fabro/workflows/develop
    "digraph Develop {}" | save -f .fabro/workflows/develop/workflow.fabro

        # fail-open: seeds CLI failure -> gate skipped, claim passes.
        expect-degraded '1: seeds exit non-zero' (claim-body-verdict {"exit_code": 1, "stdout": "", "stderr": "boom"} $scratch)

        # fail-open: non-JSON stdout.
        expect-degraded '2: non-JSON stdout' (claim-body-verdict {"exit_code": 0, "stdout": "not json"} $scratch)

        # fail-open: tracker-reported failure.
        expect-degraded '3: success:false' (claim-body-verdict {"exit_code": 0, "stdout": '{"success":false,"error":"locked"}'} $scratch)

        # fail-open: no issue row in the answer.
        expect-degraded '4: no issue object' (claim-body-verdict {"exit_code": 0, "stdout": '{"success":true}'} $scratch)

        # clean: path-free body, and a resolving rooted citation.
        expect-ok '5: path-free body' (claim-body-verdict (show-with "just words, no citations") $scratch)
        expect-ok '6: resolving workflow-relative anchor' (claim-body-verdict (show-with "see prompts/planner.md:2 for the rule") $scratch)
        expect-ok '7: resolving workflow-relative bare path' (claim-body-verdict (show-with "the prompt prompts/planner.md holds the rule") $scratch)
        expect-ok '8: resolving rooted citation' (claim-body-verdict (show-with "the publish step of .fabro/workflows/develop/workflow.fabro runs") $scratch)

        # blocking: rooted citation of a path that exists nowhere.
        let rooted = (claim-body-verdict (show-with "the gate lives in .fabro/workflows/develop/claim-gate.nu today") $scratch)
        if $rooted.outcome != "failed" { fail $"9: rooted missing citation must block, got ($rooted | to json --raw)" }
        if ($rooted.blocking | where {|b| $b.path == ".fabro/workflows/develop/claim-gate.nu"} | is-empty) { fail "9: blocking row missing the cited path" }
        if ($rooted.body | str contains "claim-gate.nu") != true { fail "9: failed verdict must echo the full body" }
        print "PASS (9: rooted missing citation blocks, body echoed)"

        # blocking: line-anchored workflow-relative citation of a file
        # that exists nowhere (the original incident class).
        let anchored = (claim-body-verdict (show-with "at prompts/gone.md:15 the mandate sits") $scratch)
        if $anchored.outcome != "failed" { fail $"10: anchored missing citation must block, got ($anchored | to json --raw)" }
        if ($anchored.blocking | where {|b| $b.path == "prompts/gone.md" and $b.line == 15} | is-empty) { fail "10: blocking row missing path:15" }
        print "PASS (10: anchored missing citation blocks)"

        # non-blocking: creation-intent window right before the path.
        expect-ok '11: creation-intent window' (claim-body-verdict (show-with "add scripts/new/gate.nu that resolves every path") $scratch)
        expect-ok '12: creation-intent (propose a sibling)' (claim-body-verdict (show-with "or propose lib/gate/new_check.rs for the same effect") $scratch)

        # non-blocking: unrooted bare token (illustrative pair).
        let unrooted = (claim-body-verdict (show-with "the diff pattern bad.md/good.md shows the flip") $scratch)
        if $unrooted.outcome != "succeeded" { fail $"13: unrooted bare token must stay advisory, got ($unrooted | to json --raw)" }
        if ($unrooted.advisory | where {|a| $a.path == "bad.md/good.md"} | is-empty) { fail "13: advisory row missing" }
        print "PASS (13: unrooted bare token advisory-only)"

        # non-blocking: out-of-range anchor (file exists, lines drifted).
        let oor = (claim-body-verdict (show-with "see pkg/a.rs:999 for the tail") $scratch)
        if $oor.outcome != "succeeded" { fail $"14: out-of-range anchor must stay advisory, got ($oor | to json --raw)" }
        if ($oor.advisory | where {|a| $a.status == "out_of_range"} | is-empty) { fail "14: advisory out_of_range row missing" }
        print "PASS (14: out-of-range anchor advisory-only)"

        # non-blocking: URL :port shapes are never path:line citations.
        expect-ok '15: URL port is not an anchor' (claim-body-verdict (show-with "the server at https://host.io:443/x answers") $scratch)

        # ── line-tip guard, pure core (fabro-006d) ─────────────────────
        # pass arm: base == tip — nothing landed since the pin.
        let same = (line-tip-verdict "abc" "abc" "ignored log text" "fabro-828c" "body")
        if $same.outcome != "succeeded" or not $same.skipped { fail $"16: base==tip must skip green, got ($same | to json --raw)" }
        print "PASS (16: base==tip skips green)"

        # pass arm: base behind, but neither the seed id nor a cited path
        # is in the range — the claim stays legal.
        let clean_range = (line-tip-verdict "abc" "def" "unrelated work\nlib/other.rs\n" "fabro-828c" "touches scripts/verify.nu")
        if $clean_range.outcome != "succeeded" or $clean_range.skipped { fail $"17: unrelated range must pass scanned, got ($clean_range | to json --raw)" }
        if ($clean_range.evidence | is-not-empty) { fail "17: no evidence expected" }
        print "PASS (17: unrelated landed work passes)"

        # bounce arm: a landed commit body names the claimed seed id.
        let seed_hit = (line-tip-verdict "abc" "def" "tracker: close fabro-828c via PR #405\n.seeds/issues.jsonl\n" "fabro-828c" "body")
        if $seed_hit.outcome != "failed" { fail $"18: landed seed id must bounce, got ($seed_hit | to json --raw)" }
        if ($seed_hit.evidence | where {|e| $e.seed_in? != null } | is-empty) { fail "18: seed_in evidence missing" }
        print "PASS (18: landed seed id bounces)"

        # bounce arm: a landed commit touched a rooted path the seed body
        # cites — the work surface already moved.
        let path_hit = (line-tip-verdict "abc" "def" "other work\nscripts/verify.nu\n" "fabro-828c" "change scripts/verify.nu per .fabro/scripts/verify.nu")
        if $path_hit.outcome != "failed" { fail $"19: landed cited path must bounce, got ($path_hit | to json --raw)" }
        if ($path_hit.evidence | where {|e| $e.path_in? != null } | is-empty) { fail "19: path_in evidence missing" }
        print "PASS (19: landed cited path bounces)"

        # fail-open arm: no range to scan.
        let no_range = (line-tip-verdict "abc" "def" "" "fabro-828c" "body")
        if $no_range.outcome != "succeeded" or not $no_range.skipped { fail $"20: empty range must skip green, got ($no_range | to json --raw)" }
        print "PASS (20: empty range fails open)"

        # end-to-end negative dry-run of the edited script (fabro-006d
        # demand): a non-git scratch dir makes the fetch fail-open, the
        # seeds CLI finds no tracker and the body gate degrades — the
        # claim-check still exits 0 and never blocks on tooling.
        let script = ($script_dir | path join "claim-check.nu")
        let raw = (do { echo "fabro-x1" | ^nu $script } | complete)
        if $raw.exit_code != 0 { fail $"21: fail-open dry-run must exit 0, got ($raw | to json --raw)" }
        if not ($raw.stderr | str contains "fail-open") { fail $"21: expected a fail-open note, got ($raw.stderr | str substring 0..200)" }
        print "PASS (21: script dry-run fails open end-to-end)"

        print "claim-check-smoke: all cases pass"
} finally {
    cd $home_dir
    rm -rf $scratch
}
# Explicit exit BEFORE the auto-invoked sourced main (stdin is empty
# here) can fail the battery — the closeout-smoke idiom.
exit 0
