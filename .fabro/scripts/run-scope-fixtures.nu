#!/usr/bin/env nu
# Run-scope fixtures (fabro-70b5 part D): proves scripts/qualitygate.nu's
# scope classification RED BOTH WAYS, for BOTH lanes — the pure
# classification (scope-violations) over the four decisive path classes,
# plus one end-to-end git fixture (run branch + checkpoint subject) for
# the base derivation the subcommand uses.
#
#   nu .fabro/scripts/run-scope-fixtures.nu
#
# Classes:
#   - product lane: loop assets (except journal + tracker file) violate;
#     repo code + the two exemptions pass.
#   - loop lane: everything outside the loop-asset set violates
#     (lib/, docs/, apps/, .agents/, .mulch/); loop assets + the tracker
#     file pass.
#   - exec-bit preservation (fabro-9569): mode-drop-paths matches only
#     100755 => 100644 drops (git's ` mode change 100755 => 100644 <path>`
#     summary shape); the git fixture proves check-mode-preservation RED
#     on an exec-bit drop and GREEN on a mode-preserving +x-gain diff.

# `source` resolves relative to THIS file's directory (anchor_check.nu
# idiom); the runtime path for re-invocation is derived at run time.
source ../../scripts/qualitygate.nu
const SCRIPT_DIR = (path self | path dirname)
let QUALITYGATE = ($SCRIPT_DIR | path join '../..' 'scripts' 'qualitygate.nu' | path expand)
let QUALITYGATE_SRC = (open --raw $QUALITYGATE)

def fail [what: string]: nothing -> nothing {
    print -e $"run-scope-fixtures: FAIL — ($what)"
    exit 1
}

def expect-violations [paths: list<string>, lane: string, expected: list<string>]: nothing -> nothing {
    let got = (scope-violations $paths $lane)
    if $got != $expected {
        fail $"lane=($lane) paths=($paths | to json -r): expected violations ($expected | to json -r), got ($got | to json -r)"
    }
}

# ── pure classification, product lane ─────────────────────────────────
# Loop assets violate (shell-staged escape hatch class — the checkpoint
# envelope does NOT see them: the product implementer has no fs_write
# pin).
expect-violations [".fabro/workflows/develop/workflow.fabro" "scripts/run.sh" "justfile" ".agents/skills/x/SKILL.md" ".seeds/other.jsonl"] "product" [".fabro/workflows/develop/workflow.fabro" "scripts/run.sh" "justfile" ".agents/skills/x/SKILL.md" ".seeds/other.jsonl"]

# .mulch passes for the PRODUCT lane on purpose: develop's lesson-capture
# contract writes the git-tracked expertise files there (ml record).
expect-violations [".mulch/expertise/rust.jsonl"] "product" []

# The two exemptions pass: hook-owned journal, tracker bookkeeping.
expect-violations [".fabro/journal/01X.jsonl" ".seeds/issues.jsonl"] "product" []

# Repo code passes untouched.
expect-violations ["lib/apps/fabro-cli/src/main.rs" "apps/fabro-web/src/x.ts" "docs/public/api.yaml" "README.md"] "product" []

# ── pure classification, loop lane ────────────────────────────────────
# Loop assets pass (the meta lane's surface); the fabro-dot snapshot dir
# passes too (fabro-9973 — mechanically derived from .fabro/workflows
# graphs, part of the loop lane's one-unit graph edit).
expect-violations [".fabro/workflows/loop/workflow.fabro" "scripts/qualitygate.nu" "justfile" ".seeds/issues.jsonl" ".fabro/journal/01X.jsonl" "lib/components/fabro-dot/src/snapshots/fabro_dot__tests__x.snap"] "loop" []

# Everything else violates (out of lane) — including lib/ paths OUTSIDE
# the snapshot dir (adjacent product code stays out of lane).
expect-violations ["lib/apps/fabro-cli/src/main.rs" "lib/components/fabro-dot/src/tests.rs" "docs/internal/x.md" "apps/fabro-web/src/x.ts" "Cargo.toml" ".agents/skills/x/SKILL.md" ".mulch/expertise/rust.jsonl" ".seeds/other.jsonl"] "loop" ["lib/apps/fabro-cli/src/main.rs" "lib/components/fabro-dot/src/tests.rs" "docs/internal/x.md" "apps/fabro-web/src/x.ts" "Cargo.toml" ".agents/skills/x/SKILL.md" ".mulch/expertise/rust.jsonl" ".seeds/other.jsonl"]

# Empty diff: green for both lanes.
expect-violations [] "product" []
expect-violations [] "loop" []

# ── pure classification, exec-bit preservation (fabro-9569) ──────────
# mode-drop-paths over `git diff --summary` line shapes: a 100755 =>
# 100644 drop matches; a +x gain (100644 => 100755), content-only
# diffs, and renames never do.
def expect-drops [lines: list<string>, expected: list<string>]: nothing -> nothing {
    let got = (mode-drop-paths $lines)
    if $got != $expected {
        fail $"mode-drop lines=($lines | to json -r): expected ($expected | to json -r), got ($got | to json -r)"
    }
}
expect-drops [" mode change 100755 => 100644 scripts/run.sh"] ["scripts/run.sh"]
expect-drops [" mode change 100644 => 100755 scripts/run.sh"] []
expect-drops [" scripts/run.sh | 2 +-" " rename scripts/a.sh => scripts/b.sh (98%)"] []
expect-drops [] []
expect-drops [" mode change 100755 => 100644 a.sh" " mode change 100644 => 100755 b.sh"] ["a.sh"]

# ── drift pin: the loop-asset set is encoded in evidence.nu
# (loop-work-path — the reviewer scope) and qualitygate.nu (loop-asset?
# — the gate scope). A silent drift between them would split reviewer
# scope from gate scope; this textual pin REDs on either side changing
# without the other (extraction into one shared module is blocked by
# nu's source-auto-invoke pitfall — this is the cheap deterministic
# substitute; the loop graph-contract smoke pins the graph side).
const PIN_PREFIXES = '[".fabro/" "scripts/"] | any {|q| $path | str starts-with $q}'
const PIN_PREFIXES_QG = '[".fabro/" "scripts/"] | any {|q| $p | str starts-with $q}'
const PIN_EXACT_EVIDENCE = '($path in ["justfile" ".seeds/issues.jsonl"])'
const PIN_EXACT_QG = '($p in ["justfile" ".seeds/issues.jsonl"])'
let evidence_src = (open --raw ($SCRIPT_DIR | path join '../workflows/develop/scripts/evidence.nu'))
if not ($evidence_src | str contains $PIN_PREFIXES) or not ($evidence_src | str contains $PIN_EXACT_EVIDENCE) {
    fail $"drift pin: evidence.nu loop-work-path set changed — realign with scripts/qualitygate.nu loop-asset? \(and the loop graph x.fs_write\)"
}
if not ($QUALITYGATE_SRC | str contains $PIN_PREFIXES_QG) or not ($QUALITYGATE_SRC | str contains $PIN_EXACT_QG) {
    fail "drift pin: qualitygate.nu loop-asset? set changed — realign with evidence.nu loop-work-path \(and the loop graph x.fs_write\)"
}
# fabro-9973: the snapshot-dir exception must live in BOTH mirrors (and
# the loop graph pins its own copy) — a one-sided exception would split
# gate scope from reviewer scope.
const PIN_SNAPSHOT_DIR = 'lib/components/fabro-dot/src/snapshots/'
if not ($evidence_src | str contains $PIN_SNAPSHOT_DIR) or not ($QUALITYGATE_SRC | str contains $PIN_SNAPSHOT_DIR) {
    fail "drift pin: the fabro-dot snapshot-dir exception (fabro-9973) must appear in BOTH evidence.nu loop-work-path and qualitygate.nu loop-asset?"
}

# ── end-to-end git fixture ────────────────────────────────────────────
# A throwaway repo on a run branch with a checkpoint-shaped commit: the
# check-run-scope path derivation must ground on the checkpoint parent
# and see exactly the staged files. Proves BOTH lanes' exit codes:
# loop-asset-only diff -> loop GREEN / product RED; repo-code diff ->
# product GREEN / loop RED. RED proven both ways, both lanes.
let FX = (mktemp -d)
cd $FX
do { ^git init -q -b main } | ignore
^git config user.email fx@fabro.sh
^git config user.name fx
"base" | save base.txt
do { ^git add -A } | ignore
do { ^git commit -qm base } | ignore
do { ^git checkout -qb fabro/run/fxrun1 } | ignore
mkdir .fabro/workflows/loop
"asset" | save .fabro/workflows/loop/asset.txt
"gate" | save .fabro/workflows/loop/gate.txt
do { ^git add -A } | ignore
# checkpoint-shaped subject: the base derivation greps exactly this mark
do { ^git commit -qm "fabro(fxrun1): implementer (succeeded)" } | ignore

def probe [lane: string]: nothing -> int {
    (do { nu $QUALITYGATE check-run-scope $lane } | complete | get exit_code)
}

let loop_rc = (probe loop)
let product_rc = (probe product)
if $loop_rc != 0 { fail $"git fixture: loop lane should be GREEN on a loop-asset-only diff \(rc=($loop_rc)\)" }
if $product_rc == 0 { fail "git fixture: product lane should be RED on a loop-asset-only diff \(rc=($product_rc)\)" }

# Flip side: repo-code-only diff -> product GREEN, loop RED.
do { ^git checkout -q main } | ignore
do { ^git checkout -qb fabro/run/fxrun2 } | ignore
"code" | save src_main.rs
mkdir src
"code" | save src/main.rs
do { ^git add -A } | ignore
do { ^git commit -qm "fabro(fxrun2): implementer (succeeded)" } | ignore
let product_rc2 = (probe product)
let loop_rc2 = (probe loop)
if $product_rc2 != 0 { fail $"git fixture: product lane should be GREEN on a repo-code-only diff \(rc=($product_rc2)\)" }
if $loop_rc2 == 0 { fail $"git fixture: loop lane should be RED on a repo-code-only diff \(rc=($loop_rc2)\)" }

# ── end-to-end git fixture, exec-bit preservation (fabro-9569) ───────
# The base commit carries a tracked executable; a run branch that drops
# its exec bit (100755 => 100644) must RED check-mode-preservation, a
# mode-preserving content edit (plus a NEW executable) must stay GREEN.
cd $FX
do { ^git checkout -q main } | ignore
"tool" | save tool.sh
^chmod +x tool.sh
do { ^git add -A } | ignore
do { ^git commit -qm base-tool } | ignore

def probe-mode []: nothing -> int {
    (do { nu $QUALITYGATE check-mode-preservation } | complete | get exit_code)
}

# Exec-bit drop -> RED.
do { ^git checkout -qb fabro/run/fxrun3 } | ignore
^chmod -x tool.sh
do { ^git add -A } | ignore
do { ^git commit -qm "fabro(fxrun3): implementer (succeeded)" } | ignore
let mode_rc = (probe-mode)
if $mode_rc == 0 { fail $"git fixture: check-mode-preservation should be RED on an exec-bit drop \(rc=($mode_rc)\)" }

# Mode-preserving content edit + a NEW executable (+x gain) -> GREEN.
do { ^git checkout -q main } | ignore
do { ^git checkout -qb fabro/run/fxrun4 } | ignore
"tool v2" | save --force tool.sh
"new" | save fresh.sh
^chmod +x fresh.sh
do { ^git add -A } | ignore
do { ^git commit -qm "fabro(fxrun4): implementer (succeeded)" } | ignore
let mode_rc2 = (probe-mode)
if $mode_rc2 != 0 { fail $"git fixture: check-mode-preservation should be GREEN on a mode-preserving diff with a +x gain \(rc=($mode_rc2)\)" }

print "run-scope-fixtures: ok — pure classification + git base derivation + exec-bit preservation, RED proven both ways both lanes"

# Sourcing qualitygate.nu imports its `def main`; nu auto-invokes it after
# the top level runs (closeout-smoke idiom) — exit explicitly so the
# battery never reaches it.
exit 0
