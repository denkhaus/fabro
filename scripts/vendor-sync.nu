#!/usr/bin/env nu
# vendor-sync (fabro-eeee, ADR-0034): move the fork crates between the
# vendored tree in this repo and the fork branches they come from.
#
#   just vendor status                 tree vs fork tip, per fork (no clone)
#   just vendor import <name>          fork tip -> vendor/<name> + PINS
#   just vendor export <name> -m ...   vendor/<name> -> fork commit + push
#
# Why this shape:
#   - The tree is a SNAPSHOT. The fork branch stays the history archive and
#     the upstream-PR vehicle; the fabro repository carries no fork history.
#   - Both directions use ONE managed clone per fork at tmp/vendor/<name>
#     (gitignored, created on demand), so a machine without the large dev
#     checkouts can still sync. The clone is a cache, never a source of
#     truth: import re-extracts the tree, export re-copies it.
#   - Provenance is vendor/PINS.toml, verified by scripts/vendor-pins-fixtures.nu
#     (`just vendor-pins`). Import and export both rewrite the section, so
#     a snapshot can never lag its recorded sha.
#   - The crate list in PINS is derived from the [patch] block in
#     Cargo.toml — the crates fabro actually consumes. A patch entry that
#     names a crate the tree does not carry fails the import.

const PINS_PATH = "vendor/PINS.toml"
const CLONE_ROOT = "tmp/vendor"

# sha256 over "<git-blob-sha>  <relpath>\n" lines, sorted by path, over the
# TRACKED tree (`git ls-files`): build outputs inside vendor/<name>/target
# must not read as a snapshot edit. The tree is staged right after import,
# so the index names exactly the snapshot's files.
def content-hash [dir: string]: nothing -> string {
    let files = (^git ls-files $dir | lines | sort)
    if ($files | is-empty) {
        error make {msg: $"content-hash: ($dir) has no staged files — stage the tree first \(git add\)"}
    }
    let lines = ($files | each {|f|
        let rel = ($f | str replace $"($dir)/" "")
        let blob = (^git hash-object --no-filters $f | str trim)
        $"($blob)  ($rel)"
    })
    let text = if ($lines | is-empty) { "" } else { ($lines | str join (char newline)) + (char newline) }
    ($text | hash sha256)
}

def pins [] {
    open $PINS_PATH
}

def section [name: string] {
    let all = (pins)
    if not ($name in ($all | columns)) {
        let known = ($all | columns | str join ', ')
        error make {msg: $"vendor-sync: no [($name)] section in ($PINS_PATH) — known: ($known)"}
    }
    $all | get $name
}

# The crates fabro consumes from a fork: names of the [patch] entries that
# point into the fork repository. The section header names the UPSTREAM url
# (that is what gets patched); the entries point at the fork's tree.
def patched-crates [upstream: string]: nothing -> list<string> {
    let lines = (open --raw Cargo.toml | lines)
    let hits = ($lines | enumerate | where {|x|
        let l = ($x.item | str trim)
        ($l | str starts-with '[patch."https://github.com/') and ($l | str contains $upstream)
    })
    if ($hits | is-empty) {
        error make {msg: $"vendor-sync: Cargo.toml has no [patch] block for ($upstream) — add the path patches before importing"}
    }
    let start = ($hits | get index.0)
    let body = ($lines | skip ($start + 1) | take while {|l|
        let t = ($l | str trim)
        ($t != '') and (not ($t | str starts-with '['))
    })
    $body
    | each {|l| ($l | str trim | split row ' = ' | first | str trim) }
    | where {|c| ($c | is-not-empty) and (not ($c | str starts-with '#')) }
    | sort
}

def render-section [name: string, e: record]: nothing -> string {
    let crates = ($e.crates | each {|c| $"    \"($c)\"," } | str join (char newline))
    [
        $"[($name)]"
        $"vendor = \"($e.vendor)\""
        $"fork = \"($e.fork)\""
        $"branch = \"($e.branch)\""
        $"fork_sha = \"($e.fork_sha)\""
        $"upstream = \"($e.upstream)\""
        $"upstream_sha = \"($e.upstream_sha)\""
        $"tree_sha = \"($e.tree_sha)\""
        $"content_sha256 = \"($e.content_sha256)\""
        $"files = ($e.files)"
        "crates = ["
        $crates
        "]"
        $"imported = \"($e.imported)\""
    ] | str join (char newline)
}

# Replace exactly one [name] section in the PINS file, keeping the header
# comment and the other sections byte-identical.
def write-section [name: string, e: record] {
    let raw = (open --raw $PINS_PATH)
    let lines = ($raw | lines)
    let header = $"[($name)]"
    let starts = ($lines | enumerate | where {|x| ($x.item | str trim) == $header })
    if ($starts | is-empty) {
        error make {msg: $"vendor-sync: ($PINS_PATH) has no ($header) section to update"}
    }
    let start = ($starts | get index.0)
    let rest = ($lines | enumerate | skip ($start + 1) | where {|x| ($x.item | str starts-with '[') })
    let end = if ($rest | is-empty) { ($lines | length) } else { $rest | get index.0 }
    let tail = ($lines | skip $end)
    let merged = ((($lines | take $start) | append ((render-section $name $e) | lines)) | append $tail)
    ($merged | str join (char newline)) + (char newline) | save --force $PINS_PATH
    print $"ok: ($PINS_PATH): updated ($header)"
}

def fork-url [fork: string]: nothing -> string {
    $"https://github.com/($fork).git"
}

def remote-sha [fork: string, branch: string]: nothing -> string {
    let out = (do { ^git ls-remote (fork-url $fork) $"refs/heads/($branch)" } | complete)
    if $out.exit_code != 0 {
        error make {msg: $"vendor-sync: cannot read ($fork) ($branch): ($out.stderr | str trim)"}
    }
    ($out.stdout | lines | first | split row "\t" | first | str trim)
}

# One managed clone per fork. Created on demand, then just fetched — the
# clone is a working cache, not a source of truth.
def ensure-clone [fork: string, branch: string]: nothing -> string {
    let dest = $"($CLONE_ROOT)/($fork | split row '/' | last)"
    if ($dest | path exists) {
        let fetch = (do { ^git -C $dest fetch --quiet origin $branch } | complete)
        if $fetch.exit_code != 0 {
            error make {msg: $"vendor-sync: fetch failed in ($dest): ($fetch.stderr | str trim)"}
        }
        let co = (do { ^git -C $dest checkout --quiet $"origin/($branch)" } | complete)
        if $co.exit_code != 0 {
            error make {msg: $"vendor-sync: checkout failed in ($dest): ($co.stderr | str trim)"}
        }
    } else {
        mkdir $CLONE_ROOT
        let url = (fork-url $fork)
        let clone = (do { ^git clone --quiet --branch $branch --single-branch $url $dest } | complete)
        if $clone.exit_code != 0 {
            error make {msg: $"vendor-sync: clone of ($url) failed: ($clone.stderr | str trim)"}
        }
    }
    $dest
}

def today []: nothing -> string {
    date now | format date '%Y-%m-%d'
}

# ---------------------------------------------------------------------------
# status — cheap drift check, no clone required
# ---------------------------------------------------------------------------
def 'main status' [] {
    let all = (pins)
    mut drift = false
    for name in ($all | columns) {
        let e = ($all | get $name)
        print $"== ($name): ($e.vendor) <- ($e.fork)#($e.fork_sha | str substring 0..11)"
        let dir = ($e.vendor | into string)
        if not ($dir | path exists) {
            print $"RED: ($name): ($dir) is missing"
            $drift = true
            continue
        }
        let actual = (content-hash $dir)
        if $actual != ($e.content_sha256 | into string) {
            print $"DRIFT: ($name): the tree differs from the snapshot recorded in PINS — run `just vendor-pins` for the details, then `just vendor export ($name)` to publish or `just vendor import ($name)` to discard"
            $drift = true
        } else {
            print $"ok: ($name): tree matches the recorded snapshot"
        }
        let tip = (remote-sha ($e.fork | into string) ($e.branch | into string))
        if $tip == ($e.fork_sha | into string) {
            print $"ok: ($name): fork branch ($e.branch) is at the recorded sha"
        } else {
            print $"AHEAD: ($name): fork branch ($e.branch) is at ($tip | str substring 0..11), the tree records ($e.fork_sha | str substring 0..11) — `just vendor import ($name)` to take it"
        }
    }
    if $drift { exit 1 }
    print "vendor status: no local drift"
}

# ---------------------------------------------------------------------------
# import — fork tip -> tree
# ---------------------------------------------------------------------------
def 'main import' [name: string, --upstream-sha: string = ''] {
    let e = (section $name)
    let fork = ($e.fork | into string)
    let branch = ($e.branch | into string)
    let dest = ($e.vendor | into string)
    let crates = (patched-crates ($e.upstream | into string))

    let clone = (ensure-clone $fork $branch)
    let sha = (^git -C $clone rev-parse HEAD | str trim)
    let tree = (^git -C $clone rev-parse 'HEAD^{tree}' | str trim)
    print $"== ($name): importing ($fork)#($sha | str substring 0..11) into ($dest)"

    rm -rf $dest
    mkdir $dest
    let extract = (do { ^git -C $clone archive --format=tar HEAD | ^tar -x -C $dest } | complete)
    if $extract.exit_code != 0 {
        error make {msg: $"vendor-sync: archive extract into ($dest) failed: ($extract.stderr | str trim)"}
    }
    # Stage the tree right away: the content hash reads the tracked set
    # (git ls-files), and run-images stages the cook context from the index.
    let stage = (do { ^git add $dest } | complete)
    if $stage.exit_code != 0 {
        error make {msg: $"vendor-sync: staging ($dest) failed: ($stage.stderr | str trim)"}
    }

    # Crate directories are not named after the crate; resolve by manifest.
    let unresolved = ($crates | where {|c|
        (do { ^rg --quiet --fixed-strings --glob Cargo.toml $"name = \"($c)\"" $dest } | complete).exit_code != 0
    })
    if ($unresolved | is-not-empty) {
        error make {msg: $"vendor-sync: ($fork) does not carry the patched crates: ($unresolved | str join ', ') — fix the [patch] block or the fork"}
    }

    let files = (^git ls-files $dest | lines | length)
    let upstream_sha = if ($upstream_sha | is-not-empty) {
        $upstream_sha
    } else {
        let recorded = ($e.upstream_sha | into string)
        let tip = (do { ^git ls-remote (fork-url ($e.upstream | into string)) refs/heads/main } | complete)
        if $tip.exit_code == 0 {
            let now = ($tip.stdout | lines | first | split row "\t" | first | str trim)
            if $now != $recorded {
                print $"note: upstream ($e.upstream) main is at ($now | str substring 0..11), PINS records ($recorded | str substring 0..11) as the validated base — kept; pass --upstream-sha to move it"
            }
        }
        $recorded
    }

    let entry = {
        vendor: $dest
        fork: $fork
        branch: $branch
        fork_sha: $sha
        upstream: ($e.upstream | into string)
        upstream_sha: $upstream_sha
        tree_sha: $tree
        content_sha256: (content-hash $dest)
        files: $files
        crates: $crates
        imported: (today)
    }
    write-section $name $entry
    print $"ok: ($name): ($files) files, ($crates | length) crates, content ($entry.content_sha256 | str substring 0..15)"
    print "next: `just vendor-pins` (provenance) and a full build against the new tree"
}

# ---------------------------------------------------------------------------
# export — tree -> fork commit
# ---------------------------------------------------------------------------
def 'main export' [
    name: string
    --message (-m): string = ''
    --dry-run
] {
    let e = (section $name)
    let fork = ($e.fork | into string)
    let branch = ($e.branch | into string)
    let dest = ($e.vendor | into string)
    if not ($dest | path exists) {
        error make {msg: $"vendor-sync: ($dest) is missing — nothing to export"}
    }
    if ($message | is-empty) and (not $dry_run) {
        error make {msg: "vendor-sync: export needs --message (the fork commit message)"}
    }

    let clone = (ensure-clone $fork $branch)
    # The clone is a scratch copy of the fork branch: make it byte-equal to
    # the tree, .git excluded.
    let sync = (do { ^rsync -a --delete --exclude '.git/' $"($dest)/" $"($clone)/" } | complete)
    if $sync.exit_code != 0 {
        error make {msg: $"vendor-sync: rsync into ($clone) failed: ($sync.stderr | str trim)"}
    }
    # The content hash and the PINS rewrite below read the tracked set; a
    # tree edited in place must be staged before it is pinned.
    let stage = (do { ^git add $dest } | complete)
    if $stage.exit_code != 0 {
        error make {msg: $"vendor-sync: staging ($dest) failed: ($stage.stderr | str trim)"}
    }

    let dirty = (^git -C $clone status --porcelain | str trim)
    if ($dirty | is-empty) {
        print $"ok: ($name): the fork branch already matches the tree — nothing to export"
        let head = (^git -C $clone rev-parse HEAD | str trim)
        if $head != ($e.fork_sha | into string) {
            print $"note: ($name): PINS records ($e.fork_sha | str substring 0..11), the branch is at ($head | str substring 0..11) — `just vendor import ($name)` to align"
        }
        return
    }

    if $dry_run {
        print ($dirty | lines | first 40 | str join (char newline))
        let count = ($dirty | lines | length)
        print $"dry-run: ($name): ($count) paths differ between ($dest) and ($fork)#($branch) — nothing committed or pushed"
        return
    }

    let commit = (do { ^git -C $clone add -A; ^git -C $clone commit --quiet -m $message } | complete)
    if $commit.exit_code != 0 {
        error make {msg: $"vendor-sync: commit in ($clone) failed: ($commit.stderr | str trim)"}
    }
    let push = (do { ^git -C $clone push --quiet origin $"HEAD:($branch)" } | complete)
    if $push.exit_code != 0 {
        error make {msg: $"vendor-sync: push to ($fork) ($branch) failed: ($push.stderr | str trim)"}
    }

    let sha = (^git -C $clone rev-parse HEAD | str trim)
    let tree = (^git -C $clone rev-parse 'HEAD^{tree}' | str trim)
    let entry = ($e | merge {
        fork_sha: $sha
        tree_sha: $tree
        content_sha256: (content-hash $dest)
        files: (^git ls-files $dest | lines | length)
        crates: (patched-crates ($e.upstream | into string))
        imported: (today)
    })
    write-section $name $entry
    print $"ok: ($name): exported to ($fork)#($sha | str substring 0..11)"
    print "next: `just vendor-pins` to confirm the snapshot and the fork commit agree"
}

def main [] {
    print "vendor-sync (fabro-eeee, ADR-0034) — the vendored fork crates"
    print ""
    print "  nu scripts/vendor-sync.nu status                 tree vs fork tip, per fork"
    print "  nu scripts/vendor-sync.nu import <name>          fork tip -> vendor/<name> + PINS"
    print "  nu scripts/vendor-sync.nu export <name> -m '...' vendor/<name> -> fork commit + push"
    print ""
    print $"forks: (open $PINS_PATH | columns | str join ', ')"
}
