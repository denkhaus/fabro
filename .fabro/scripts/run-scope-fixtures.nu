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

# `source` resolves relative to THIS file's directory (anchor_check.nu
# idiom); the runtime path for re-invocation is derived at run time.
source ../../scripts/qualitygate.nu
const SCRIPT_DIR = (path self | path dirname)
let QUALITYGATE = ($SCRIPT_DIR | path join '../..' 'scripts' 'qualitygate.nu' | path expand)

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
expect-violations [".fabro/workflows/develop/workflow.fabro" "scripts/run.sh" "justfile" ".agents/skills/x/SKILL.md" ".mulch/expertise/rust.jsonl" ".seeds/other.jsonl"] "product" [".fabro/workflows/develop/workflow.fabro" "scripts/run.sh" "justfile" ".agents/skills/x/SKILL.md" ".mulch/expertise/rust.jsonl" ".seeds/other.jsonl"]

# The two exemptions pass: hook-owned journal, tracker bookkeeping.
expect-violations [".fabro/journal/01X.jsonl" ".seeds/issues.jsonl"] "product" []

# Repo code passes untouched.
expect-violations ["lib/apps/fabro-cli/src/main.rs" "apps/fabro-web/src/x.ts" "docs/public/api.yaml" "README.md"] "product" []

# ── pure classification, loop lane ────────────────────────────────────
# Loop assets pass (the meta lane's surface).
expect-violations [".fabro/workflows/loop/workflow.fabro" "scripts/qualitygate.nu" "justfile" ".seeds/issues.jsonl" ".fabro/journal/01X.jsonl"] "loop" []

# Everything else violates (out of lane).
expect-violations ["lib/apps/fabro-cli/src/main.rs" "docs/internal/x.md" "apps/fabro-web/src/x.ts" "Cargo.toml" ".agents/skills/x/SKILL.md" ".mulch/expertise/rust.jsonl" ".seeds/other.jsonl"] "loop" ["lib/apps/fabro-cli/src/main.rs" "docs/internal/x.md" "apps/fabro-web/src/x.ts" "Cargo.toml" ".agents/skills/x/SKILL.md" ".mulch/expertise/rust.jsonl" ".seeds/other.jsonl"]

# Empty diff: green for both lanes.
expect-violations [] "product" []
expect-violations [] "loop" []

# ── end-to-end git fixture ────────────────────────────────────────────
# A throwaway repo on a run branch with a checkpoint-shaped commit: the
# check-run-scope path derivation must ground on the checkpoint parent
# and see exactly the staged files. Proves BOTH lanes' exit codes:
# loop-asset-only diff -> loop GREEN / product RED; repo-code diff ->
# product GREEN / loop RED. RED proven both ways, both lanes.
let FX = (mktemp -d)
cd $FX
do { git init -q -b main } | ignore
git config user.email fx@fabro.sh
git config user.name fx
"base" | save base.txt
do { git add -A } | ignore
do { git commit -qm base } | ignore
do { git checkout -qb fabro/run/fxrun1 } | ignore
mkdir .fabro/workflows/loop
"asset" | save .fabro/workflows/loop/asset.txt
"gate" | save .fabro/workflows/loop/gate.txt
do { git add -A } | ignore
# checkpoint-shaped subject: the base derivation greps exactly this mark
do { git commit -qm "fabro(fxrun1): implementer (succeeded)" } | ignore

def probe [lane: string]: nothing -> int {
    (do { nu $QUALITYGATE check-run-scope $lane } | complete | get exit_code)
}

let loop_rc = (probe loop)
let product_rc = (probe product)
if $loop_rc != 0 { fail $"git fixture: loop lane should be GREEN on a loop-asset-only diff \(rc=($loop_rc)\)" }
if $product_rc == 0 { fail "git fixture: product lane should be RED on a loop-asset-only diff \(rc=($product_rc)\)" }

# Flip side: repo-code-only diff -> product GREEN, loop RED.
do { git checkout -q main } | ignore
do { git checkout -qb fabro/run/fxrun2 } | ignore
"code" | save src_main.rs
mkdir src
"code" | save src/main.rs
do { git add -A } | ignore
do { git commit -qm "fabro(fxrun2): implementer (succeeded)" } | ignore
let product_rc2 = (probe product)
let loop_rc2 = (probe loop)
if $product_rc2 != 0 { fail $"git fixture: product lane should be GREEN on a repo-code-only diff \(rc=($product_rc2)\)" }
if $loop_rc2 == 0 { fail $"git fixture: loop lane should be RED on a repo-code-only diff \(rc=($loop_rc2)\)" }

print "run-scope-fixtures: ok — pure classification + git base derivation, RED proven both ways both lanes"

# Sourcing qualitygate.nu imports its `def main`; nu auto-invokes it after
# the top level runs (closeout-smoke idiom) — exit explicitly so the
# battery never reaches it.
exit 0
