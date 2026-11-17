#!/usr/bin/env nu
# Dev-artifact prune (thin target of `just dev-prune`; wired into
# `just image-release` since the mirtuell remote flow replaced `just up`
# as the main path — the old `just up` pipeline ran `just clean` at its
# start and end; `just clean` chains this script too). Scope: HOST-side
# temp artifacts only.
#
#   1. target/ GC — delegates to clean-target.nu 'stale' (drops
#      incremental/ dirs unused >= 6h; cargo never GCs, measured 52 GB/day).
#      The docker release build does NOT use target/ (it builds in the
#      fabro-docker-cargo-target-<arch> volume), so this never invalidates
#      the release-build cache.
#   2. tmp/docker-context/ — staging dirs from `cargo dev docker-build`
#      (regenerated per build; safe to drop).
#   3. tmp/vendor/*/target — build caches inside the managed fork clones.
#      The clones are history archives, never build environments; 2026-11-17
#      finding: 59 GB petri + 3.9 GB sandbox-driver of target/ waste.
#      Always dropped (not age-gated — a target/ inside the archive is
#      garbage at any age).
#   4. stale top-level tmp/ artifacts (>= 7 days) — run dumps, review dirs,
#      db snapshots; regenerable via `fabro dump` / session reruns. Keep
#      set: vendor (clone sources), just-up.lock (the up pipeline lock),
#      rig-tools (workbench tools).
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

    # 3. build caches inside the managed fork clones (tmp/vendor/*/target):
    #    the clones are history archives — building inside them leaves
    #    target/ waste that no GC sweeps (2026-11-17 finding: 59 GB petri
    #    + 3.9 GB sandbox-driver). Always dropped, not age-gated: a target/
    #    inside the archive is garbage at any age.
    let vend = 'tmp/vendor'
    if ($vend | path exists) {
        let targets = (glob $"($vend)/*/target")
        if ($targets | is-empty) {
            print "dev-artifact-prune: no tmp/vendor/*/target build caches present"
        }
        for t in $targets {
            let size = (dir-size $t)
            if $dry_run {
                print $"dev-artifact-prune: would remove ($t) \(($size)\)"
            } else {
                rm -rf $t
                print $"dev-artifact-prune: removed ($t) \(($size) freed\)"
            }
        }
    }

    # 4. stale top-level tmp/ artifacts (>= 7 days): run dumps, review
    #    dirs, db snapshots — regenerable via `fabro dump` / session
    #    reruns. Keep set: vendor (clone sources), just-up.lock (the up
    #    pipeline lock), rig-tools (workbench tools).
    let keep = ['vendor' 'just-up.lock' 'rig-tools']
    let cutoff = ((date now) - 7day)
    let stale_tmp = (ls tmp
        | where modified < $cutoff
        | where {|e| ($e.name | path basename) not-in $keep })
    if ($stale_tmp | is-empty) {
        print "dev-artifact-prune: no stale tmp artifacts (>= 7 days)"
    }
    for e in $stale_tmp {
        let size = (dir-size $e.name)
        if $dry_run {
            print $"dev-artifact-prune: would remove ($e.name) \(($size)\)"
        } else {
            rm -rf $e.name
            print $"dev-artifact-prune: removed ($e.name) \(($size) freed\)"
        }
    }

    # Guard rail: surface (never touch) the protected docker caches so an
    # operator reading the log sees they were considered and skipped.
    print "dev-artifact-prune: protected (untouched): docker volumes fabro-docker-*, images fabro-toolchain/fabro-runner, builder cache"
}
