#!/usr/bin/env nu
# Dev-artifact prune (thin target of `just dev-prune`; wired into
# `just image-release` since the mirtuell remote flow replaced `just up`
# as the main path — the old `just up` pipeline ran `just clean` at its
# start and end). Scope: HOST-side temp artifacts only.
#
#   1. target/ GC — delegates to clean-target.nu 'stale' (drops
#      incremental/ dirs unused >= 6h; cargo never GCs, measured 52 GB/day).
#      The docker release build does NOT use target/ (it builds in the
#      fabro-docker-cargo-target-<arch> volume), so this never invalidates
#      the release-build cache.
#   2. tmp/docker-context/ — staging dirs from `cargo dev docker-build`
#      (regenerated per build; safe to drop).
#
# HARD NON-GOALS (never prune here — each costs ~30 min to rebuild):
#   - docker volumes fabro-docker-cargo-{registry,target,tools,rustup,zig}-*
#     (the release-build cache, AGENTS.md docker-build approach)
#   - docker images fabro-toolchain / ghcr.io/denkhaus/fabro-toolchain:<tag>,
#     fabro-runner:* (content-hash tagged; the toolchain bake incl. the
#     cargo-chef warm layers, #264, is ~30 min)
#   - docker builder layer cache (NO `docker builder prune`, NO
#     `docker system prune` — both destroy the above)
# Host-side run-image cleanup on the mirtuell docker host is a separate
# concern (sandbox GC seed) and deliberately out of scope here.

# `path self` is parse-time only — anchor as a const (same pattern as
# planner-preflight.nu).
const SCRIPT_DIR = (path self | path dirname)

def dir-size [path: string]: nothing -> string {
    if ($path | path exists) {
        (^du -sh $path | lines | first | split column "\t" size _name | get 0.size)
    } else { "" }
}

def main [--dry-run] {
    print $"dev-artifact-prune: (if $dry_run { 'DRY RUN — ' } else { '' })scope: target/ stale GC + tmp/docker-context staging"

    # 1. target/ GC (clean-target.nu handles its own reporting + dry-run)
    if $dry_run {
        let out = (do { nu ($SCRIPT_DIR | path join 'clean-target.nu') stale --dry-run } | complete)
        print ($out.stdout | str trim -r -c "\n" | lines | last 6 | str join (char newline))
    } else {
        nu ($SCRIPT_DIR | path join 'clean-target.nu') stale
    }

    # 2. docker-build staging dirs
    let ctx = 'tmp/docker-context'
    if ($ctx | path exists) {
        let size = (dir-size $ctx)
        if $dry_run {
            print $"dev-artifact-prune: would remove ($ctx) \(($size)\)"
        } else {
            rm -rf $ctx
            print $"dev-artifact-prune: removed ($ctx) \(($size) freed\)"
        }
    } else {
        print "dev-artifact-prune: no tmp/docker-context present"
    }

    # Guard rail: surface (never touch) the protected docker caches so an
    # operator reading the log sees they were considered and skipped.
    print "dev-artifact-prune: protected (untouched): docker volumes fabro-docker-*, images fabro-toolchain/fabro-runner, builder cache"
}
