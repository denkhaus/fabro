#!/usr/bin/env nu
# Bootstrap the workspace toolchain beyond what mise provides.
# Called by `just bootstrap` from the [run.prepare] step (after `mise install`).

const NPM_CLIS = [
    "@os-eco/seeds-cli@0.5.15"
    "@os-eco/mulch-cli@0.10.7"
]

# Every tool the develop workflow relies on; each must resolve and --version.
def required-tools []: nothing -> list<string> {
    [sd ml go just nu]
}

# Run `<tool> --version` and print its version line. False (with a message)
# when the tool is missing or errors — bun-installed shims count on PATH.
def verify-tool [name: string]: nothing -> bool {
    let res = (do { ^$name --version } | complete)
    if $res.exit_code != 0 {
        print $"missing or broken tool: ($name)"
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

    let broken = (required-tools | where {|tool| not (verify-tool $tool) })
    if ($broken | is-not-empty) {
        error make --unspanned {msg: $"bootstrap failed: missing tools \(($broken | str join ', ')\)"}
    }
    print "bootstrap ok"
}
