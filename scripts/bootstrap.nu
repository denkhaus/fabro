#!/usr/bin/env nu
# Bootstrap the workspace toolchain beyond what mise provides.
# Called by `just bootstrap` from the [run.prepare] step (after `mise install`).

const NPM_CLIS = [
    "@os-eco/seeds-cli@0.5.15"
    "@os-eco/mulch-cli@0.10.7"
]

# Every tool the develop workflow relies on, with the command that proves
# it. `go` has no --version flag (exit 2); it only answers `go version`.
def required-tools []: nothing -> list<record<tool: string, check: list<string>>> {
    [
        {tool: sd,  check: [--version]}
        {tool: ml,  check: [--version]}
        {tool: go,  check: [version]}
        {tool: just, check: [--version]}
        {tool: nu,  check: [--version]}
    ]
}

# Run the tool's check command and print its version line. False (with a
# message) when the tool is missing or errors — bun-installed shims count
# on PATH; arguments stay separate values, never an interpolated command.
def verify-tool [tool: string, check: list<string>]: nothing -> bool {
    let res = (do { ^$tool ...$check } | complete)
    if $res.exit_code != 0 {
        print $"missing or broken tool: ($tool)"
        false
    } else {
        print ($res.stdout | str trim)
        true
    }
}

def main []: nothing -> nothing {
    # npm-distributed CLIs land via `bun install -g` because mise's aube/npm
    # backend demands interactive confirmation; the runner image pins
    # BUN_INSTALL to a directory already on PATH, so the shims are visible
    # to every stage.
    bun install -g ...$NPM_CLIS

    let broken = (required-tools | where {|t| not (verify-tool $t.tool $t.check) })
    if ($broken | is-not-empty) {
        error make --unspanned {msg: $"bootstrap failed: missing tools \(($broken.tool | str join ', ')\)"}
    }
    print "bootstrap ok"
}
