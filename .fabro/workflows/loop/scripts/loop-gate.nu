#!/usr/bin/env nu
# Loop-lane deterministic gate (fabro-70b5 part C): the loop tester's
# validation battery. This is the "deterministic protection" the meta
# lane restores — schema-parseability and contractual soundness of the
# loop's own machinery, WITHOUT the product gate's compile tier.
#
# Battery (each check green = exit 0; first red stops, qualitygate
# style: literal external commands in `do { ^cmd } | complete`):
#   1. `just validate-workflows` — petri admission + the fork's lint
#      rules on EVERY workflow graph (a loop run that breaks any graph's
#      parseability or envelope lints REDs here; prebuilt fabro CLI
#      inside run sandboxes, no cargo build).
#   2. `just lint-nu` — parse check + interpolated-regex scan of every
#      repo nu script (root scripts/ AND all workflow assets).
#   3. `nu .fabro/scripts/prompt-lint.nu` — prompt/graph/toml literal
#      hygiene: unresolvable seed ids, drifted justfile anchors,
#      provenance literals, nu -c quoting traps.
#   4. `nu scripts/qualitygate.nu check-run-scope loop` — the run diff
#      touches ONLY loop assets (.fabro/**, scripts/**, justfile,
#      .seeds/issues.jsonl); a loop run editing lib/, docs/, apps/ is
#      out of lane and REDs.
#   5. `nu scripts/qualitygate.nu check-mode-preservation` — no exec-bit
#      drop (100755 => 100644) rides the run diff (fabro-9569); granting
#      +x stays green.
#   6. `nu scripts/qualitygate.nu check-dot-snapshot` (fabro-9973) — ONLY
#      when the run diff changes a workflow graph under
#      .fabro/workflows/**: run the fabro-dot checked-in-workflows
#      snapshot test (one crate, no product battery). A graph-shape
#      change that ships without its accepted snapshot costs a full
#      dogfood-gate cycle to catch; the drift REDs here instead, with
#      the acceptance instruction.
#   7. Rust fmt tier, ONLY when the diff touches .rs files — a pure
#      safety net: loop seeds should never touch Rust (the run-scope
#      check already refuses lib/**; scripts/** is nu-only today). If a
#      .rs file ever appears in scope, format discipline still applies.
#
# NOT here (deliberate, fabro-70b5): the product gate's clippy/nextest
# compile tier — the product lane's tester owns that; the loop lane
# never compiles product code — EXCEPT the one bounded probe above
# (fabro-9973): the fabro-dot snapshot tier compiles exactly one crate
# and only when the diff changes a workflow graph.

const PINNED_TOOLCHAIN = "nightly-2026-04-14"

def run-base [] {
    let run_id = (
        ^git branch --show-current | str trim
        | parse --regex 'fabro/run/(?P<id>[^/]+)$'
        | get -o id.0
        | default ''
    )
    if ($run_id | is-empty) {
        # Interactive/human invocation: diff the working tree against
        # HEAD (uncommitted changes) — the same fallback qualitygate.nu
        # uses.
        return "HEAD"
    }
    let subject_mark = $"fabro\(($run_id)\):"
    let checkpoints = (^git log --format=%H --fixed-strings --grep $subject_mark | lines | compact)
    if ($checkpoints | is-empty) {
        return "HEAD"
    }
    ^git rev-parse $"($checkpoints | last)^" | str trim
}

def check [name: string, res: record]: nothing -> bool {
    print $"== ($name) =="
    if ($res.stdout | str trim | is-not-empty) { print ($res.stdout | str trim -r -c "\n") }
    if $res.exit_code != 0 {
        if ($res.stderr | str trim | is-not-empty) { print -e ($res.stderr | str trim -r -c "\n") }
        print -e $"loop-gate: ($name) RED"
        return false
    }
    true
}

def main []: nothing -> nothing {
    let base = (run-base)

    if not (check 'validate-workflows (every graph: petri admission + fork lint rules)' (do { ^just validate-workflows } | complete)) { exit 1 }
    if not (check 'lint-nu (every nu script)' (do { ^just lint-nu } | complete)) { exit 1 }
    if not (check 'prompt-lint (literal hygiene)' (do { ^nu .fabro/scripts/prompt-lint.nu } | complete)) { exit 1 }
    if not (check 'run-scope (loop lane: diff touches only loop assets)' (do { ^nu scripts/qualitygate.nu check-run-scope loop } | complete)) { exit 1 }
    if not (check 'mode-preservation (no exec-bit drop through the run diff)' (do { ^nu scripts/qualitygate.nu check-mode-preservation } | complete)) { exit 1 }
    if not (check 'dot-snapshot (fabro-dot checked-in-workflows when the diff touches a workflow graph)' (do { ^nu scripts/qualitygate.nu check-dot-snapshot } | complete)) { exit 1 }

    # Rust fmt tier — only when the diff actually touches .rs files.
    let rs = (do { ^git diff --name-only $base } | complete | get stdout | lines | compact | where {|p| $p | str ends-with '.rs'})
    if ($rs | is-not-empty) {
        print $"== rust fmt — diff touches \(($rs | length)\) .rs file\(s\) =="
        let res = (do { ^cargo $'+($PINNED_TOOLCHAIN)' fmt --check --all } | complete)
        if $res.exit_code != 0 {
            print ($res.stdout | str trim -r -c "\n" | lines | last 20)
            print ($res.stderr | str trim -r -c "\n")
            print -e 'loop-gate: rust fmt RED'
            exit 1
        }
        print 'format clean'
    } else {
        print '== rust fmt skipped — no .rs files in diff =='
    }

    print 'LOOP GATE GREEN'
}
