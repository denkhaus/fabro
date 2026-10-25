#!/usr/bin/env nu
# Shared loop-asset spec-path seam: `loop-asset?` (the loop lane's
# fs allow-list predicate) and `spec-named-loop-paths` (the spec-named
# loop-asset paths a seed body mandates in the run diff) lived as
# private defs of scripts/qualitygate.nu; the claim gate needs the
# SAME extraction to emit the list into stage output (the implementer's
# pre-tester spec-paths self-check), and sourcing qualitygate whole
# collides on its `def main`. One file, both callers, no drift — the
# anchor_check.nu idiom (shared scripts over copies).
#
# `source` resolves against THIS file's directory.
source ../../workflows/develop/scripts/anchor_check.nu

# Pure: one path against the loop-lane allow-list (mirrors the loop
# implementer's x.fs_write — keep the pair in sync; the loop
# graph-contract smoke pins the graph side).
export def loop-asset? [p: string]: nothing -> bool {
    # the ONE lib/ exception: the fabro-dot checked-in-workflows
    # snapshot under this directory is mechanically derived from
    # .fabro/workflows graphs — accepting it (`cargo insta accept`)
    # is part of the loop lane's one-unit graph edit, not a
    # product-code edit. Every other lib/ path stays out of lane.
    if ($p | str starts-with "lib/components/fabro-dot/src/snapshots/") { return true }
    let prefix_hit = ([".fabro/" "scripts/"] | any {|q| $p | str starts-with $q})
    ($prefix_hit) or ($p in ["justfile" ".seeds/issues.jsonl" ".seeds/config.yaml"])
}

# Pure: expand every `{a,b,...}` comma group of a brace-form citation
# into its alternatives (cartesian across groups, recursive on the
# remainder). A token without a comma group returns itself untouched —
# only an enumeration is an expansion; `{record}` stays literal and is
# dropped by the brace filter downstream. Exported for the fixture
# battery (the pure-function pin).
export def expand-brace-groups [p: string]: nothing -> list<string> {
    let m = ($p | parse --regex '^(?<pre>[^{}]*)\{(?<alts>[^{}]*,[^{}]*)\}(?<post>.*)$')
    if ($m | is-empty) { return [$p] }
    let row = ($m | first)
    ($row.alts | split row ',' | where {|a| ($a | str trim) != ''})
    | each {|a| expand-brace-groups ($row.pre + $a + $row.post)}
    | flatten
}

# Pure: the spec-named loop-asset paths of a seed body — extraction is
# deliberately conservative so a named path is always diffable:
#   - only loop-asset paths (the loop implementer's envelope);
#   - globs/wildcards and directory tokens drop: the diff lists files,
#     a glob or prefix can never match one — keeping them would
#     false-RED every spec that names a root sweep (`.fabro/**`);
#   - `.fabro/journal/` and `.fabro/revisions/` paths drop: journal and
#     revision citations in a spec body are PROVENANCE (the Origin/Basis
#     lines point at the reviewer run that filed the residual or the
#     revision record it authored), never an edit mandate — both are
#     append-only records no pass edits, so keeping the citation
#     was a permanent false RED on every residual seed;
#   - brace-form citations expand BEFORE the path filter runs
#     (denkhaus/work#6): a `{a,b,c}` comma group cannot tokenize under
#     the path-character class (the incident: claim_check emitted ONE
#     path for a `{planner,implementer,reviewer}.md` citation and the
#     one-unit gate under-constrained the whole run), so brace-form
#     tokens are extracted with a brace-aware class and each comma
#     group expands to its full alternatives — every expanded path is
#     then subject to the SAME filters as a literal citation, with the
#     brace literal as the creation-intent needle (an `Add
#     prompts/{new,old}.md` window drops both alternatives, not just
#     the literally-named ones);
#   - creation-named paths drop (the claim gate's shared
#     creation-intent window, anchor_check.nu): `Add scripts/new.nu`
#     is a proposed target, not an edit mandate, while `extend
#     scripts/old.nu` stays enforced;
#   - paths that resolve as directories in THIS worktree drop (the
#     battery runs from the checkout root).
export def spec-named-loop-paths [desc: string]: nothing -> list<string> {
    if ($desc | is-empty) { return [] }
    let clean = ($desc | str replace --all --regex '\S+://\S+' ' ')
    # Literal citations: the path-character class (no braces — a comma
    # group cannot tokenize under it). Each candidate carries its
    # literal token as the creation-intent needle.
    let plain = ($clean
        | parse --regex '(?<![A-Za-z0-9._/-])(?<p>[A-Za-z0-9._/-]+/[A-Za-z0-9._/-]+)'
        | get -o p
        | default []
        | each {|p| {path: ($p | str trim -r -c '.'), lit: $p}})
    # Brace-form citations: the SAME boundary, one wider character class
    # that admits braces and commas; only tokens carrying a `{a,b}`
    # comma group survive, expanded BEFORE the filters run. Trailing
    # sentence punctuation (`,` `.`) is trimmed off the expanded paths
    # exactly like the literal trailing dot.
    let braced = ($clean
        | parse --regex '(?<![A-Za-z0-9._/-])(?<p>[A-Za-z0-9._{},/-]+/[A-Za-z0-9._{},/-]+)'
        | get -o p
        | default []
        | where {|t| ($t | parse --regex '\{[^{}]*,[^{}]*\}' | is-not-empty)}
        | each {|t|
            expand-brace-groups $t | each {|e|
                {path: ($e | str trim -r -c ',' | str trim -r -c '.'), lit: $t}
            }
        }
        | flatten)
    ($plain | append $braced)
    | uniq
    | where {|r|
        ((not ($r.path | str starts-with '.fabro/journal/')) and (not ($r.path | str starts-with '.fabro/revisions/')) and (not ($r.path | str contains '*')) and (not ($r.path | str contains '?')) and (not ($r.path | str contains '{')) and (not ($r.path | str ends-with '/')) and (loop-asset? $r.path))
    }
    | where {|r| ($r.path | path type) != 'dir'}
    | where {|r| (not (creation-named? $clean $r.path)) and (not (creation-named? $clean $r.lit))}
    | get path
    | uniq
}
