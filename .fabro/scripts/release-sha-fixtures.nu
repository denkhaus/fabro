#!/usr/bin/env nu
# release-sha fixtures (fabro-06da): a RELEASE tag must name the PUSHABLE
# line tip, never the GitButler workspace commit. Exactly one policy site
# owns that derivation (.fabro/scripts/line-tip-sha.nu); this battery keeps
# every other script out of the business, pins that the two release scripts
# call it, and proves the scanner has teeth.
#
# The offending pattern is assembled from fragments on purpose: written
# literally in this file the scanner below would flag this file itself (the
# same self-reference trap prompt-lint avoids by excluding its fixtures).

const PATTERN_A = 'rev-parse'
const PATTERN_B = '--short'

# Files allowed to run `git rev-parse --short`, each with its reason. A new
# entry is a deliberate edit: the point of the battery is that deriving a
# short sha is a decision, not an accident.
const ALLOWED = [
    # The policy site itself: falls back to git HEAD in a plain checkout.
    '.fabro/scripts/line-tip-sha.nu'
    # Run-LOCAL diff bases inside a run's own checkout (the run's HEAD or a
    # checkpoint ref), never a release tag. loop uses these develop copies.
    '.fabro/workflows/develop/scripts/evidence.nu'
    '.fabro/workflows/develop/scripts/closeout.nu'
    # This battery: it assembles the pattern from fragments to scan for it
    # and derives nothing itself.
    '.fabro/scripts/release-sha-fixtures.nu'
]

# Every *.nu file under `dirs` that derives a short sha and is not allowed.
def unauthorised-short-sha [dirs: list] {
    $dirs
    | each {|dir|
        # Both levels: `**` alone skips a file sitting directly in `dir`.
        (glob $"($dir)/*.nu") | append (glob $"($dir)/**/*.nu")
    }
    | flatten
    | uniq
    | where {|f|
        let allowed = ($ALLOWED | any {|a| $f | str ends-with $a})
        if $allowed { false } else {
            let text = (open --raw $f)
            ($text | str contains $PATTERN_A) and ($text | str contains $PATTERN_B)
        }
    }
}

def main [] {
    # 1) The tree: only the allow-listed files derive a short sha.
    let bad = (unauthorised-short-sha ['scripts' '.fabro'])
    if ($bad | is-not-empty) {
        print "RED: unauthorised short-sha derivation (add the file to ALLOWED with a reason, or call .fabro/scripts/line-tip-sha.nu):"
        $bad | each {|f| print $"  ($f)"}
        exit 1
    }
    print "ok: no unauthorised short-sha derivation"

    # 2) Teeth: the same scanner finds the pattern in a throwaway file.
    let tmp = (mktemp -d)
    # Concatenated, never interpolated: inside $"..(..).." nushell would
    # EVALUATE the parentheses and plant the derived sha instead of the
    # pattern (the same evaluation trap as backticks in a bash argument).
    let planted_text = ('let sha = (git ' + $PATTERN_A + ' ' + $PATTERN_B + ' HEAD)')
    $planted_text | save --force $"($tmp)/planted.nu"
    if ((unauthorised-short-sha [$tmp] | is-empty)) {
        print "RED: the scanner cannot see a planted derivation - the tree check above proves nothing"
        exit 1
    }
    print "ok: the scanner finds a planted derivation"

    # 3) The release scripts call the ONE policy site (the fix pin): an edit
    #    that drops the call fails here instead of shipping a workspace-commit
    #    tag that no push publishes.
    for script in ['scripts/run-images.nu' 'scripts/image-release.nu'] {
        if not ((open --raw $script) | str contains '.fabro/scripts/line-tip-sha.nu') {
            print $"RED: ($script) does not call the line-tip policy site"
            exit 1
        }
    }
    print "ok: both release scripts call the line-tip policy site"

    print "release-sha-fixtures: green"
}
