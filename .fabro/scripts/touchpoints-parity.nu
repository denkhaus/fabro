#!/usr/bin/env nu
# Touchpoints parity (fabro-de32, arch-gate sprint 9): the fork's two-pin
# rule — every durable fork feature keeps a row in the touchpoints registry
# AND a fork-only pin file — was enforced only by the develop reviewer
# prompt (reviewer.md item 6b). This battery machine-checks both sides:
#
#   forward:  every GIT-TRACKED fork-only pin file (lib/**/tests/fork_*.rs,
#             lib/**/src/**/fork_*_tests.rs, lib/**/src/**/fork_*.rs) must be
#             named, by basename, somewhere in the registry — a pin without a
#             row is invisible to the merge walk (gaps found at filing:
#             fabro-petri fork_exec_guard.rs x2, fabro-llm fork_structured.rs).
#   reverse:  literal .rs paths a row names (crate-relative or lib/-rooted,
#             no glob/brace characters) must exist on disk — a row whose pin
#             is gone is drift the other way. Unresolvable tokens (prose,
#             globs) are skipped, never flagged.
#
# The inventory is git-tracked files ONLY: a sibling agent's uncommitted
# experiment must not red this battery — committed work carries its row by
# definition of the two-pin rule.
#
# Registry path: .agents/skills/merge-upstream/references/touchpoints.md —
# fabro-63fc step 1 relocates the registry to a neutral home; that change
# must move REGISTRY_REL with it (one definition, one-line edit).
#
#   nu .fabro/scripts/touchpoints-parity.nu

const SCRIPT_DIR = (path self | path dirname)
const REGISTRY_REL = '.agents/skills/merge-upstream/references/touchpoints.md'
# Scope guards: both sides must be substantial, or the battery would green
# vacuously after a path-form or inventory change (skill-parity's guard
# pattern, fabro-6538).
const MIN_ROWS = 10
const MIN_PINS = 20

def fail [what: string]: nothing -> nothing {
    print -e $"touchpoints-parity: FAIL — ($what)"
    exit 1
}

let root = ($SCRIPT_DIR | path join '../..' | path expand)
let registry_path = ($root | path join $REGISTRY_REL)
if not ($registry_path | path exists) {
    fail $"registry missing: ($REGISTRY_REL)"
}
let registry = (open --raw $registry_path)

let rows = (
    $registry
    | lines
    | where {|line| ($line | str starts-with '|') and not ($line | str starts-with '|--')}
)
if ($rows | length) < $MIN_ROWS {
    fail $"registry has only ($rows | length) rows — expected at least ($MIN_ROWS); is ($REGISTRY_REL) still the registry?"
}

# --- forward: tracked fork-only pin files ------------------------------
let git = (do { ^git ls-files } | complete)
if $git.exit_code != 0 {
    fail $"git ls-files failed: ($git.stderr | str substring 0..200)"
}
let pins = (
    $git.stdout
    | lines
    | where {|f|
        ($f | str starts-with 'lib/') and ($f | str ends-with '.rs') and (
            ($f | path basename | str starts-with 'fork_') or ($f | str contains '/fork_')
        )
    }
)
if ($pins | length) < $MIN_PINS {
    fail $"pin inventory has only ($pins | length) files — expected at least ($MIN_PINS); the fork_ glob drifted"
}
let unpinned = ($pins | where {|f| ($f | path basename) not-in $registry })

# --- reverse: literal row paths must exist ------------------------------
let crate_dirs = (
    ['lib/apps' 'lib/components' 'lib/foundation' 'test']
    | each {|d|
        ls ($root | path join $d)
        | get name
        | each {|n| {crate: ($n | path basename), dir: ($n | path relative-to $root)} }
    }
    | flatten
)
def resolve [token: string, crates: list<record>, root: string]: nothing -> list<string> {
    if ($token | str starts-with 'lib/') or ($token | str starts-with 'test/') {
        [$token]
    } else {
        let parts = ($token | path split)
        let hit = ($crates | where crate == $parts.0)
        if ($hit | is-empty) { [] } else {
            let rest = ($parts | skip 1 | str join '/')
            [($hit.0.dir | path join $rest) ($hit.0.dir | path join 'src' $rest) ($hit.0.dir | path join 'tests' $rest)]
        }
    }
}
let dead = (
    $registry
    | lines
    | each {|line| $line | parse --regex '(?P<p>[\w\-./]+\.rs)' | get -o p }
    | flatten
    | compact
    | uniq
    | where {|t| not ($t | str contains '*') and not ($t | str contains '?') and not ($t | str contains '{') }
    | each {|t| {token: $t, candidates: (resolve $t $crate_dirs $root)} }
    | where {|r| ($r.candidates | is-not-empty) and ($r.candidates | where {|c| ($root | path join $c | path exists)} | is-empty) }
    | get token
)

# --- verdict ------------------------------------------------------------
if ($unpinned | is-not-empty) {
    for f in $unpinned {
        print -e $"touchpoints-parity: pin file without a registry row: ($f) — add a row for it in ($REGISTRY_REL) or rename the file"
    }
}
if ($dead | is-not-empty) {
    for t in $dead {
        print -e $"touchpoints-parity: registry names a path that does not exist: ($t) — the row drifted"
    }
}
if ($unpinned | is-empty) and ($dead | is-empty) {
    print $"touchpoints-parity: green — ($pins | length) tracked fork-only pin files all named in ($REGISTRY_REL); ($rows | length) rows; no dead literal paths"
} else {
    print -e "touchpoints-parity: registry and pin inventory diverged — the two-pin rule is broken in the direction(s) above"
    exit 1
}
