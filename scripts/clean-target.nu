#!/usr/bin/env nu
# GC stale host build artifacts from target/ (thin launcher: `just clean-target`).
#
# Cargo never garbage-collects: every crate x fingerprint x edit round leaves
# incremental/<crate>-<hash>/ and deps/*-<hash>.rlib behind forever (measured:
# 52 GB in one day, 937 incremental dirs for 150 crates). The docker release
# build (just up) does NOT use target/ — it builds in the
# fabro-docker-cargo-target-<arch> volume — so cleaning here never invalidates
# that cache.
#
# Modes:
#   stale (default) — drop incremental/ dirs unused for >= 6h (safe: the
#                     state is regenerable; worst case one non-incremental
#                     recompile on the next edit round)
#   sweep           — stale, then GC deps/ via cargo-sweep --time 24
#                     (requires: cargo install cargo-sweep)
#   all             — full cargo clean (last resort)

# Human-readable size of a path via `du -sh`; empty string when missing.
def dir-size [path: string]: nothing -> string {
    if ($path | path exists) {
        ^du -sh $path
        | lines
        | first
        | split column "\t" size _name
        | get 0.size
    } else {
        ""
    }
}

def drop-stale-incremental [dry_run: bool]: nothing -> int {
    const INC = "target/debug/incremental"
    if not ($INC | path exists) {
        return 0
    }
    let cutoff = ((date now) - 6hr)
    let stale = (ls $INC | where type == dir and modified < $cutoff)
    if $dry_run {
        print $"clean-target: would drop ($stale | length) incremental dirs older than 6h"
        for d in ($stale | get name) {
            print $"  would remove ($d)"
        }
    } else {
        for d in ($stale | get name) {
            # A concurrently running build may touch old dirs; never abort on one.
            try { rm --recursive --force $d } catch { |err|
                print -e $"clean-target: could not remove ($d): ($err.msg)"
            }
        }
        print $"clean-target: dropped ($stale | length) incremental dirs older than 6h"
    }
    ($stale | length)
}

def main [
    mode: string = "stale"   # stale | sweep | all
    --dry-run(-d)            # report what would be removed, delete nothing
]: nothing -> nothing {
    if not ("target" | path exists) {
        print "clean-target: no target/ — nothing to clean"
        return
    }
    let before = (dir-size "target")
    match $mode {
        "stale" | "sweep" => {
            drop-stale-incremental $dry_run
            if $mode == "sweep" {
                if ((which cargo-sweep | length) == 0) {
                    print -e "clean-target: cargo-sweep not installed: cargo install cargo-sweep"
                    exit 1
                }
                if $dry_run {
                    ^cargo sweep --time 24 --dry-run
                } else {
                    ^cargo sweep --time 24
                }
            }
        }
        "all" => {
            if $dry_run {
                print "clean-target: would run cargo clean"
            } else {
                ^cargo clean
            }
        }
        _ => {
            print -e $"clean-target: unknown mode '($mode)' — valid: stale, sweep, all"
            exit 1
        }
    }
    let after = (dir-size "target")
    let after_label = (if ($after | is-empty) { "gone" } else { $after })
    print $"target: ($before) -> ($after_label)"
}
