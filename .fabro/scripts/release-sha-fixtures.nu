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

# The policy site, imported for its PURE decision function (fabro-ed65).
# `use` (not `source`): this file has a main, and `source` would auto-invoke
# it; `use` pulls only the exported def.
use ./line-tip-sha.nu [tip-verdict]

# `path self` is parse-time only (mx-8eb3ee): anchor it in a const.
const POLICY_SCRIPT = (path self | path dirname | path join 'line-tip-sha.nu')

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

    # 4) The release tip must be the PUBLISHED tip — equality, not
    #    reachability (fabro-ed65): GitButler rewrites shas on push, so a
    #    locally-created commit stays an ancestor of the tip while the tip
    #    moved on. The pure function is pinned offline.
    if (tip-verdict 'abc123456789' 'abc123456789') != '' {
        print "RED: tip-verdict rejects a sha that IS the published tip"
        exit 1
    }
    if ((tip-verdict 'abc123456789' 'def123456789') | str length) == 0 {
        print "RED: tip-verdict accepts a sha that is not the published tip — the fork.8 class (an image tagged with an unpushed sha) would pass"
        exit 1
    }
    if ((tip-verdict 'abc123456789' '') | str length) == 0 {
        print "RED: tip-verdict accepts an unreadable remote tip — a release must fail closed, never assume"
        exit 1
    }
    print "ok: tip-verdict demands the published tip and fails closed on an unreadable one"

    # 5) Both release scripts take the sha (ONE resolution per release) and
    #    demand the published tip when they resolve it themselves.
    for script in ['scripts/run-images.nu' 'scripts/image-release.nu'] {
        # BEHAVIORAL, not textual: `--help` prints the flags the script really
        # declares, so a rationale comment naming them cannot satisfy the pin
        # (the first, textual version of this check survived a mutation that
        # deleted the real call).
        let helped = (do { ^nu $script --help } | complete)
        if not ($helped.stdout | str contains '--sha') {
            print $"RED: ($script) does not declare --sha — the release would resolve the line tip twice (fabro-ed65)"
            exit 1
        }
        # ...and its own resolution must demand the PUBLISHED tip. The error
        # messages elsewhere in these files deliberately avoid this exact
        # call form, so the check cannot be satisfied by a message string.
        let code = (open --raw $script | lines | each {|l| $l | split row '#' | first } | str join (char nl))
        if not ($code | str contains 'line-tip-sha.nu --require-published') {
            print $"RED: ($script) resolves the line tip without the published-tip guard"
            exit 1
        }
    }
    print "ok: both release scripts declare --sha and guard their own resolution"

    # 4b) The ONE resolution lives in the justfile recipe: it must resolve the
    #     tip once and pass the SAME value to both release scripts.
    let recipe = (open --raw justfile | lines | each {|l| $l | split row '#' | first } | str join (char nl))
    if not ($recipe | str contains 'line-tip-sha.nu --require-published') {
        print "RED: the image-release recipe no longer resolves the published tip"
        exit 1
    }
    let shared = (($recipe | split row '--sha "$TIP"' | length) - 1)
    if $shared < 2 {
        print $"RED: the image-release recipe passes --sha to ($shared) release scripts, expected 2 — the two images could tag different shas"
        exit 1
    }
    print "ok: the image-release recipe resolves the tip once and shares it with both release scripts"

    # 6) The pin wrapper refuses a tag whose image manifest is missing
    #    (fork.8: the parity gate passed while GHCR had no such tag).
    let pinned = (do { ^nu .fabro/scripts/pin-toolchain.nu 'deadbeefcafe' --dry-run } | complete)
    if $pinned.exit_code == 0 {
        print "RED: pin-toolchain accepted a tag with no GHCR manifest — the environment would point at an unpullable image"
        exit 1
    }
    print "ok: pin-toolchain refuses a tag with no image manifest"

    # 7) Behavioral: the published-tip guard refuses a tip no push published
    #    and passes once the branch is pushed (throwaway repos, offline).
    let tmp = (mktemp -d)
    let work = ($tmp | path join 'work')
    let bare = ($tmp | path join 'bare.git')
    mkdir $work
    ^git init -q --bare $bare
    ^git -C $work init -q
    "x" | save --force ($work | path join 'f.txt')
    ^git -C $work add f.txt
    ^git -C $work -c user.email=b@e.invalid -c user.name=battery commit -q -m init
    ^git -C $work remote add origin $bare
    let unpushed = (do { ^nu -c $"cd '($work)'; nu '($POLICY_SCRIPT)' --require-published" } | complete)
    if $unpushed.exit_code == 0 {
        print "RED: the guard released a tip that origin does not have — the fork.8 class would ship again"
        exit 1
    }
    ^git -C $work branch -M denkhaus
    ^git -C $work push -q -u origin denkhaus
    let pushed = (do { ^nu -c $"cd '($work)'; nu '($POLICY_SCRIPT)' --require-published" } | complete)
    if $pushed.exit_code != 0 {
        print $"RED: the guard refuses a PROPERLY pushed tip — the release path would be unusable: ($pushed.stderr | str trim)"
        exit 1
    }
    print "ok: the published-tip guard refuses an unpushed tip and passes a pushed one"

    print "release-sha-fixtures: green"
}
