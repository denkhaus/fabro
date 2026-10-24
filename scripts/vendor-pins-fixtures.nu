#!/usr/bin/env nu
# Vendor provenance pin (fabro-eeee, ADR-0034): vendor/<name> is a SNAPSHOT
# of the fork commit recorded in vendor/PINS.toml. This battery proves the
# snapshot still is that tree, and that the vendored crates really resolve
# from the tree instead of a git source:
#
#   1. content_sha256 — recomputed from the tree on every run (no network):
#      sha256 over the sorted lines "<git-blob-sha>  <relpath>\n".
#   2. tree_sha — the fork commit's git tree object; verified against a fork
#      clone when one is reachable (fail-open with a note when it is not, so
#      a checkout-less machine still runs the battery).
#   3. every crate listed in a section must appear in Cargo.lock WITHOUT a
#      git source — a vendored crate that silently resolves from git again
#      is exactly the drift this catches.
#
# A vendor bump is a deliberate edit of vendor/PINS.toml plus the import
# commit; `just vendor import <name>` writes both.

const PINS_PATH = "vendor/PINS.toml"
const LOCK_PATH = "Cargo.lock"

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

# One record per lock entry: {name, git}. Duplicate names are kept — a crate
# that resolves BOTH from the tree and from git is the drift this catches.
def locked-entries [] {
    mut entries = []
    mut name = ''
    mut git = false
    let lines = ((open --raw $LOCK_PATH | lines) | append '')
    for line in $lines {
        if ($line | str starts-with 'name = ') {
            $name = ($line | str replace 'name = ' '' | str trim | str replace --all '"' '')
            $git = false
        } else if ($line | str starts-with 'source = "git+') {
            $git = true
        } else if ($line | str trim | is-empty) {
            if ($name | is-not-empty) { $entries = ($entries | append {name: $name, git: $git}) }
            $name = ''
            $git = false
        }
    }
    $entries
}

# Fork clones that may be reachable on this machine (checked read-only):
# the managed sync clones first (tmp/vendor/<name> — vendor-sync's cache),
# then the classic dev checkouts.
def clone-path [fork: string]: nothing -> string {
    let name = ($fork | split row '/' | last)
    let candidates = [$"tmp/vendor/($name)" $"../($name)" $"../../($name)" $"($env.HOME? | default '')/dev/($name)"]
    let found = ($candidates | where {|p| ($p | path exists) and (($p | path join '.git') | path exists) })
    if ($found | is-empty) { '' } else { $found | first }
}

def main [] {
    let pins = (open $PINS_PATH)
    let locked = (locked-entries)
    mut red = false

    for name in ($pins | columns) {
        let entry = ($pins | get $name)
        let dir = ($entry.vendor | into string)
        print $"== ($name): ($dir) <- ($entry.fork)#($entry.fork_sha | str substring 0..11)"

        if not ($dir | path exists) {
            print $"RED: ($name): vendor directory ($dir) is missing"
            $red = true
            continue
        }

        let actual = (content-hash $dir)
        if $actual != ($entry.content_sha256 | into string) {
            print $"RED: ($name): vendor tree content is ($actual), PINS records ($entry.content_sha256) — the tree was edited or re-imported without updating PINS"
            $red = true
        } else {
            print $"ok: ($name): content_sha256 matches ($entry.files) files"
        }

        let clone = (clone-path ($entry.fork | into string))
        if ($clone | is-empty) {
            print $"note: ($name): no fork clone reachable — tree_sha ($entry.tree_sha) not re-verified here, fail-open"
        } else {
            let probe = (do { ^git -C $clone cat-file -e $"($entry.fork_sha)^{tree}" } | complete)
            if $probe.exit_code != 0 {
                print $"note: ($name): clone ($clone) does not carry ($entry.fork_sha | str substring 0..11) — tree_sha not re-verified, fail-open; fetch the fork to verify"
            } else {
                let tree = (^git -C $clone rev-parse $"($entry.fork_sha)^{tree}" | str trim)
                if $tree != ($entry.tree_sha | into string) {
                    print $"RED: ($name): fork ($entry.fork_sha | str substring 0..11) has tree ($tree), PINS records ($entry.tree_sha)"
                    $red = true
                } else {
                    print $"ok: ($name): tree_sha matches the fork commit"
                }
            }
        }

        for crate in ($entry.crates | default []) {
            let matches = ($locked | where name == $crate)
            if ($matches | is-empty) {
                print $"RED: ($name): crate ($crate) is not in Cargo.lock — the vendored set and the lock disagree"
                $red = true
            } else if ($matches | any {|m| $m.git }) {
                print $"RED: ($name): crate ($crate) still resolves from a GIT source in Cargo.lock — the path patch is not in effect"
                $red = true
            }
        }
        print $"ok: ($name): ($entry.crates | default [] | length) crates resolve from the tree"
    }

    # Teeth: a PINS file that parsed to nothing would make every loop above
    # trivially green.
    if (($pins | columns) | length) < 2 {
        print $"RED: ($PINS_PATH) names only (($pins | columns) | length) forks — the parse is broken or a section was lost"
        $red = true
    }

    if $red { exit 1 }
    print "vendor-pins: green"
}
