#!/usr/bin/env nu
# Skill-parity fixtures (fabro-6538): the rust-style-guide skill ships as
# two trees — .fabro/skills/rust-style-guide (CANONICAL, run-facing: run
# agents load it via x.skills=discover and the reviewer contract depends
# on it) and .agents/skills/rust-style-guide (the local-session mirror).
# They were byte-identical by accident and nothing checked parity: an
# edit to one tree silently forked style guidance per audience. This
# battery is the deterministic hash-compare net the loop-asset tier
# executes (run-images.nu's `open --raw | hash sha256` pattern): both
# trees must expose the SAME relative file set and the SAME per-file
# sha256. RED names every divergence and the resync direction — .fabro
# is canonical, so resync is always copy .fabro -> .agents.
#
#   nu .fabro/scripts/skill-parity-fixtures.nu

const SCRIPT_DIR = (path self | path dirname)
const CANONICAL_REL = '.fabro/skills/rust-style-guide'
const MIRROR_REL = '.agents/skills/rust-style-guide'

# Relative path + sha256 for every file under one tree (sorted; the same
# walk on both sides makes list equality the parity verdict).
def tree-hashes [root: string]: nothing -> list<record<rel: string, sha: string>> {
    glob $"($root)/**/*"
    | where {|p| ($p | path type) == 'file'}
    | each {|p| {rel: ($p | path relative-to $root), sha: (open --raw $p | hash sha256)}}
    | sort-by rel
}

def fail [what: string]: nothing -> nothing {
    print -e $"skill-parity: FAIL — ($what)"
    exit 1
}

let root = ($SCRIPT_DIR | path join '../..' | path expand)
let canon = (tree-hashes ($root | path join $CANONICAL_REL))
let mirror = (tree-hashes ($root | path join $MIRROR_REL))

# Scope guard: an empty or missing tree is itself the loudest divergence
# — a deleted canonical or mirror must never read as a vacuous green.
if ($canon | is-empty) {
    fail $"canonical tree empty or missing: ($CANONICAL_REL)"
}
if ($mirror | is-empty) {
    fail $"mirror tree empty or missing: ($MIRROR_REL)"
}

let canon_rels = ($canon | get rel)
let mirror_rels = ($mirror | get rel)
let missing = ($canon_rels | where {|r| $r not-in $mirror_rels})
let extra = ($mirror_rels | where {|r| $r not-in $canon_rels})
# Inner join on rel: files present in both trees but with different
# content — the silent-fork class the seed names.
let diverged = (
    $canon
    | join ($mirror | rename --column {sha: mirror_sha}) rel
    | where {|r| $r.sha != $r.mirror_sha}
    | get rel
)

if ($missing | is-not-empty) {
    for m in $missing { print -e $"skill-parity: in canonical but missing from mirror: ($MIRROR_REL)/($m)" }
}
if ($extra | is-not-empty) {
    for e in $extra { print -e $"skill-parity: in mirror but missing from canonical: ($CANONICAL_REL)/($e)" }
}
if ($diverged | is-not-empty) {
    for d in $diverged { print -e $"skill-parity: sha256 diverged: ($d) — ($CANONICAL_REL) is canonical; resync ($MIRROR_REL) from it" }
}
if ($missing | is-empty) and ($extra | is-empty) and ($diverged | is-empty) {
    print $"skill-parity: green — ($canon_rels | length) files sha256-identical; canonical: ($CANONICAL_REL); mirror: ($MIRROR_REL)"
} else {
    print -e "skill-parity: trees diverged — .fabro is canonical; copy changed files .fabro -> .agents and re-run"
    exit 1
}
