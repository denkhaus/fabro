#!/usr/bin/env nu
# The smoke harness (denkhaus/work#1109): the per-facade shim state
# fixtures, the run-shaped temp git repo builder, the fail-log marker
# gate, and the real-parser argv pin driver existed as ~200 embedded
# lines in closeout-smoke.nu; every stage-D/G smoke (the adjudicator
# fast path first) needs the same machinery. This module owns it; no
# smoke re-implements the shapes.
#
# The fail gate: nu 0.115 catches `exit` inside `try` (e2e arms live in
# one), so a bare exit-1 fail would print FAIL and still end green —
# the battery reads exit codes. Every fail lands in a marker file whose
# path rides the environment (a def cannot close over a runtime `let`);
# the closing gate turns any recorded fail into a real non-zero exit.

# The fail-log path for THIS smoke, from the environment marker.
# `start-fail-log` returns the path; the CALLER binds it into the
# environment at its own top level (nu scopes env changes inside defs —
# a def's load-env never reaches the caller).
export def fail-log-path []: nothing -> string {
    $env.SMOKE_FAIL_LOG? | default ''
}

# Create the fail-log marker file and return its path. Call ONCE, at
# the script's top level, and bind the result into the environment:
#   load-env {SMOKE_FAIL_LOG: (start-fail-log 'my-smoke')}
# (nu scopes env changes inside defs; the top-level load-env persists.)
export def start-fail-log [prefix: string]: nothing -> string {
    mktemp -t $"($prefix)-fails.XXXXXX"
}

# Record one failure: print it and append it to the marker file, then
# exit 1 (the battery's read is the marker, not the exit — see above).
export def fail [what: string]: nothing -> nothing {
    print -e $"smoke: FAIL — ($what)"
    $"($what)\n" | save --append (fail-log-path)
    exit 1
}

# End the fail gate: any recorded fail turns into a real non-zero exit;
# a clean (or absent) marker file is removed. Call ONCE, after every
# arm.
export def end-fail-gate []: nothing -> nothing {
    let log = (fail-log-path)
    if ($log | is-empty) { return }
    if ((try { open --raw $log } catch { '' }) | str trim | is-not-empty) {
        exit 1
    }
    rm -f $log
}

# The env record pointing a lane script at the shim: FABRO_BIN fakes the
# binary, FABRO_SERVER names a dead transport (the shim never dials it),
# FABRO_WORK_SHIM_STATE names the state file.
export def shim-env [shim: string, state: string]: nothing -> record {
    {FABRO_BIN: $shim, FABRO_SERVER: "http://127.0.0.1:1", FABRO_WORK_SHIM_STATE: $state}
}

# Write one full shim state (the caller owns the shape; fresh-work-state
# is the common one-item starting point).
export def write-shim-state [path: string, state: record]: nothing -> nothing {
    $state | to json --raw | save -f $path
}

# The one-open-item shim state the closeout-class smokes start from.
export def fresh-work-state [path: string, title: string]: nothing -> nothing {
    write-shim-state $path {items: [
        {itemKey: "denkhaus/work#1", title: $title, body: "the item body", state: "open", labels: []}
    ], claims: [], nextKey: 8001, log: []}
}

# A temp git repo shaped like a RUN checkout: branch fabro/run/<run_id>,
# an initial commit, then a checkpoint commit whose subject carries the
# run-id marker (run-base anchors on it) and whose diff carries
# `impl_text`. Returns the repo path; every git call is explicit about
# its directory (no cd side effect) and the caller owns cd/cleanup (the
# `finally` idiom).
export def run-shaped-repo [run_id: string, impl_text: string]: nothing -> string {
    let repo = (mktemp -d -t smoke-run-repo.XXXXXX)
    git -C $repo init -q -b $"fabro/run/($run_id)" .
    git -C $repo config user.email smoke@localhost
    git -C $repo config user.name smoke-harness
    echo initial | save -f ($repo | path join base.txt)
    git -C $repo add base.txt
    git -C $repo commit -qm "initial"
    $impl_text | save -f ($repo | path join impl.txt)
    git -C $repo add impl.txt
    git -C $repo commit -qm $"fabro\(($run_id)\): the fixture implementing checkpoint"
    $repo
}

# Drive one CLI argv against a dead server and fail when the output
# names a clap parse error: the shim is position-blind (nu binds
# declared flags wherever they appear), so only the REAL parser can pin
# an argv grammar ('unexpected argument' is a parse error, everything
# else is the expected transport failure).
export def pin-argv [bin: string, dead_server: string, shape: list<string>]: nothing -> nothing {
    let run = (do { ^$bin ...$shape } | complete)
    let combined = ($run.stdout + $run.stderr)
    if ($combined | str contains 'unexpected argument') {
        fail $"an argv drifted out of the real parser's grammar: ($shape | str join ' ') — ($combined | str substring 0..240)"
    }
    if ($combined | str contains 'unrecognized subcommand') {
        fail $"the binary predates a verb — build the tree CLI \(cargo build -p fabro-cli\) or fix PATH: ($combined | str substring 0..240)"
    }
}
