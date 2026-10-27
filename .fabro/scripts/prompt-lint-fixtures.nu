#!/usr/bin/env nu
# prompt-lint fixture battery (fabro-cb5c): the SYNTHETIC_IDS_MARKER in
# prompt-lint.nu suspends ONE check (resolvable seed ids) for ONE file, so
# the suspension itself needs a pin. This battery runs the REAL lint — no
# re-implementation — inside throwaway roots in the system temp dir: the
# resolvable-id tier through the REAL lint's census-file seam
# (PROMPT_LINT_CENSUS_FILE, staged by new-root below): the gate host
# carries no forge client and no network, so the battery supplies the
# known legacy ids itself — the real migrated id below resolves, a dead
# fixture id reds, and the fixture files are the only lint targets in
# scope. The lint's own fail-open (census unavailable -> tier suspended)
# is not pinned here; this battery pins the tier LIVE.
#
#   nu .fabro/scripts/prompt-lint-fixtures.nu
#
# Contract (fabro-cb5c acceptance + the standards review of that change):
#   1. an unresolvable id in an UNMARKED file reds, and the red names it;
#   2. the marker silences its own battery file only — an unmarked SIBLING
#      file in the same run is still checked;
#   3. the marker is honored ONLY in a `-smoke.nu`/`-fixtures.nu` battery
#      (a marker in any other file is itself an error — the silent
#      de-gate bound);
#   4. a resolvable id beside the marker stays quiet, AND the note names
#      the ids whose check was skipped (auditable, not silent);
#   5. every other check stays live on a marked file (a drifted justfile
#      anchor still reds).
#   6. check 9's vendor-CLI scope covers the LOOP lane's prompt assets
#      (denkhaus/work#8): a vendor CLI in a loop prompt reds — narrowing
#      the check back to develop-only would leave that fixture green and
#      RED this battery — a vendor CLI in a loop SCRIPT stays out of
#      scope (the scripts/ exclusion is deliberate), and a clean loop
#      prompt stays silent.

const LINT = (path self | path dirname | path join 'prompt-lint.nu')
# Marker, dead id and anchor are built by concatenation so THIS file's own
# source neither declares the marker (the battery is a lint target itself
# and must stay unmarked) nor carries a `fabro-xxxx` literal that would not
# resolve, nor a real justfile anchor.
const MARKER = '# prompt-lint: ' + 'synthetic-seed-ids'
const DEAD = 'fabro-' + 'dead'
const REAL = 'fabro-' + 'cb5c'
const ANCHOR = 'justfile' + ':1'

def fail [what: string]: nothing -> nothing {
    print -e $"prompt-lint-fixtures: FAIL — ($what)"
    exit 1
}

def new-root [base: string, name: string]: nothing -> string {
    let root = ($base | path join $name)
    mkdir ($root | path join '.fabro' 'scripts')
    # deterministic census: stage the known legacy ids for the lint's
    # census-file seam — no tracker store, no gh, no network. Only the
    # real migrated id resolves; the DEAD concatenation never lands here.
    ($REAL + "\n") | save --force ($root | path join 'census.txt')
    $root
}

def write-fixture [root: string, file: string, body: string]: nothing -> nothing {
    $body | save --force ($root | path join '.fabro' 'scripts' $file)
}

def run-lint [root: string]: nothing -> record {
    # route the lint at the root's staged census through the seam, so the
    # resolvable-id tier is LIVE in every check below, deterministically
    let census = ($root | path join 'census.txt')
    let res = (do { with-env {PROMPT_LINT_CENSUS_FILE: $census} { ^nu -c $"cd '($root)'; nu '($LINT)'" } } | complete)
    {code: $res.exit_code, out: ($res.stdout + $res.stderr)}
}

def main [] {
    let base = (mktemp -d)

    # 1. unmarked + unresolvable -> red, naming the id
    let r1 = (new-root $base 'unmarked')
    write-fixture $r1 'plain-fixtures.nu' $"# fixture\n# ($DEAD)\n"
    let one = (run-lint $r1)
    if $one.code == 0 { fail $"an unresolvable id in an unmarked file passed the lint: ($one.out)" }
    if not ($one.out | str contains $DEAD) { fail $"the red does not name ($DEAD): ($one.out)" }
    print 'ok: an unresolvable id in an unmarked battery reds'

    # 2. marked battery silenced; an unmarked sibling stays checked
    let r2 = (new-root $base 'mixed')
    write-fixture $r2 'marked-fixtures.nu' $"# fixture\n($MARKER)\n# ($DEAD)\n"
    let two_marker_only = (run-lint $r2)
    if $two_marker_only.code != 0 { fail $"the marker did not silence its own file: ($two_marker_only.out)" }
    print 'ok: the marker silences its own battery file'
    write-fixture $r2 'sibling-fixtures.nu' $"# fixture, no marker\n# ($DEAD)\n"
    let two_with_sibling = (run-lint $r2)
    if $two_with_sibling.code == 0 { fail 'an unmarked sibling was silenced by another file marker' }
    if not ($two_with_sibling.out | str contains 'sibling-fixtures.nu') { fail $"the sibling red does not name the sibling file: ($two_with_sibling.out)" }
    print 'ok: the marker does not leak to sibling files'

    # 3. the marker is only honored in a battery file
    let r3 = (new-root $base 'nonsmoke')
    write-fixture $r3 'notabattery.nu' $"# fixture\n($MARKER)\n# ($DEAD)\n"
    let three = (run-lint $r3)
    if $three.code == 0 { fail $"a marker outside a battery file de-gated the lint: ($three.out)" }
    if not ($three.out | str contains 'only honored') { fail $"the marker misuse is not reported as such: ($three.out)" }
    if not ($three.out | str contains $DEAD) { fail $"a real id in a mis-marked file was not checked: ($three.out)" }
    print 'ok: the marker is rejected outside a battery file'

    # 4. a resolvable id beside the marker stays quiet, and is named in the note
    let r4 = (new-root $base 'real')
    write-fixture $r4 'marked-fixtures.nu' $"# fixture\n($MARKER)\n# ($DEAD)\nfabro-('cb5c')\n"
    let four = (run-lint $r4)
    if $four.code != 0 { fail $"a resolvable id beside a marker red the lint: ($four.out)" }
    if not ($four.out | str contains $REAL) { fail $"the skip note does not name the unchecked ids: ($four.out)" }
    print 'ok: the skip note names the ids it stopped checking'

    # 5. other checks stay live on a marked file: drifted justfile anchor
    let r5 = (new-root $base 'anchors')
    write-fixture $r5 'marked-fixtures.nu' $"# fixture\n($MARKER)\n# ($ANCHOR)\n"
    "all:\n# a comment\n" | save --force ($r5 | path join 'justfile')
    let five = (run-lint $r5)
    if $five.code == 0 { fail $"a drifted justfile anchor passed on a marked file: ($five.out)" }
    if not ($five.out | str contains 'comment line') { fail $"the anchor red is not the drifted-anchor finding: ($five.out)" }
    print 'ok: the justfile-anchor check stays live on a marked file'

    # 6. check 9 scope covers the LOOP lane's prompt assets: a vendor
    # CLI in a loop prompt reds (this is the pin — a narrowing back to
    # develop-only leaves this fixture green and REDs this battery), a
    # vendor CLI in a loop SCRIPT stays out of scope by design, and a
    # clean loop prompt stays silent.
    let r6 = (new-root $base 'loopscope')
    mkdir ($r6 | path join '.fabro' 'workflows' 'loop' 'prompts')
    mkdir ($r6 | path join '.fabro' 'workflows' 'loop' 'scripts')
    # Vendor literals are built by concatenation so THIS battery file
    # never carries a live vendor command form itself.
    let VENDOR = 'seeds ' + 'list'
    $"# loop prompt fixture\nrun the command `($VENDOR)` and see rot.\n" | save --force ($r6 | path join '.fabro' 'workflows' 'loop' 'prompts' 'planner.md')
    let six_red = (run-lint $r6)
    if $six_red.code == 0 { fail $"a vendor CLI in a loop prompt passed the lint: ($six_red.out)" }
    if not ($six_red.out | str contains 'tracker-CLI') { fail $"the loop-prompt red is not the vendor-CLI finding: ($six_red.out)" }
    print 'ok: a vendor CLI in a loop prompt asset reds'
    rm ($r6 | path join '.fabro' 'workflows' 'loop' 'prompts' 'planner.md')
    $"# tool\n# ($VENDOR)\n" | save --force ($r6 | path join '.fabro' 'workflows' 'loop' 'scripts' 'tool-fixtures.nu')
    "# clean loop prompt\n" | save --force ($r6 | path join '.fabro' 'workflows' 'loop' 'prompts' 'planner.md')
    let six_green = (run-lint $r6)
    if $six_green.code != 0 { fail $"the scripts-excluded vendor literal or a clean loop prompt red the lint: ($six_green.out)" }
    if ($six_green.out | str contains 'tracker-CLI') { fail $"a loop-script vendor literal was flagged despite the scripts/ exclusion: ($six_green.out)" }
    print 'ok: loop scripts stay out of scope and a clean loop prompt stays silent'

    print 'prompt-lint-fixtures: green — 6 checks, loop vendor-CLI scope pinned'
}
