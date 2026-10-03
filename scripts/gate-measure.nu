#!/usr/bin/env nu
# Cold/incremental gate timing for the fabro workspace (fabro-5453 phase 1).
# Self-contained: clones from the read-only mount (no host target/ leakage)
# into /src, measures with CARGO_TARGET_DIR=/tmp/target-fresh. Style follows
# scripts/qualitygate.nu (retired lab exemplar, archived tag): literal external
# commands, sections, explicit failure blocks.

def timed [label: string, block: closure]: nothing -> nothing {
    let start = (date now)
    let res = (do $block | complete)
    let secs = ((date now) - $start)
    print $"ELAPSED ($label): ($secs)"
    if $res.exit_code != 0 {
        print $"FAILED ($label):"
        print ($res.stderr | str trim -r -c "\n" | lines | last 15)
        exit 1
    }
}

def main []: nothing -> nothing {
    let cpus = (^nproc | str trim)
    print $"=== gate timing, cpus=($cpus), target=(($env.CARGO_TARGET_DIR? | default 'default')) ==="

    ^git config --global --add safe.directory '*'
    if ($"/src/Cargo.toml" | path exists) {
        print "reuse existing /src clone"
    } else {
        timed "clone (read-only mount -> /src)" { ^git clone --quiet /src-ro /src }
    }
    cd /src

    timed "build --workspace (kalt)" { ^cargo build --workspace }
    timed "clippy -p fabro-workflow (inkrementell)" { ^cargo clippy -p fabro-workflow --all-targets -- -D warnings }
    timed "nextest -p fabro-workflow (inkrementell)" { ^cargo nextest run -p fabro-workflow }
    timed "fmt --check --all" { ^cargo fmt --check --all }
    print "=== Messung fertig ==="
}
