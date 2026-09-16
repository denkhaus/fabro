#!/usr/bin/env nu
# Smoke test for closeout.nu's Dockerfile-touch warning (fabro-6f6e).
# Exercises the pure glob/filter logic — dockerfile-hits over a path
# list — without git; the full-script path is a manual invocation check
# (documented on the seed). The `source` const resolves against THIS
# file's directory, so the script runs from any cwd:
#   nu .fabro/workflows/develop/scripts/closeout-smoke.nu

const CLOSEOUT = "closeout.nu"
source $CLOSEOUT

def fail [what: string]: nothing -> nothing {
    print -e $"closeout-smoke: FAIL — ($what)"
    exit 1
}

# Positives: every Dockerfile* segment shape the warning must catch.
let hits = (dockerfile-hits [
    ".fabro/Dockerfile.toolchain"
    ".fabro/Dockerfile"
    "Dockerfile"
    "apps/web/Dockerfile.dev"
])
if $hits != [".fabro/Dockerfile.toolchain" ".fabro/Dockerfile" "Dockerfile" "apps/web/Dockerfile.dev"] {
    fail $"positives mismatch: ($hits | to json -r)"
}

# Negatives: lowercase, prefix-only, and unrelated paths stay silent.
# (Note `Dockerfile-notes.md` WOULD match: its segment starts with
# `Dockerfile`, which is exactly the glob `Dockerfile*` — a true
# positive, not a false one.)
let clean = (dockerfile-hits [
    "dockerfile"
    "lib/main.rs"
    "apps/fabro-web/src/x.ts"
    "a/Docker/keep.rs"
    "docs/about-Dockerfile.md"
])
if ($clean | is-not-empty) {
    fail $"negatives matched: ($clean | to json -r)"
}

# Empty input degrades to empty — no warning, byte-identical close.
if (dockerfile-hits []) != [] { fail "empty input not empty" }

# --- Closure-discipline pre-close check (fabro-02c4) -----------------
# Token extraction: distinctive >=4-char tokens survive; function-word
# stopwords, short tokens, and duplicates drop.
let toks = (demand-tokens "Closeout closure discipline: sd close only when the seed demand is visible in the run diff, else a documented reason is mandatory")
for expected in ["closeout" "demand" "visible" "diff"] {
    if not ($toks | any {|t| $t == $expected }) { fail $"demand-tokens dropped '($expected)'" }
}
for banned in [seed when else only this with from] {
    if ($toks | any {|t| $t == $banned }) { fail $"demand-tokens kept stopword '($banned)'" }
}
if ($toks | any {|t| ($t | str length) < 4 }) { fail "demand-tokens kept a <4-char token" }

# Visible demand: matching token in a non-empty patch -> close proceeds.
if not (demand-visible ["closeout" "retry"] "diff --git a/.fabro/x b/.fabro/x
+closeout gate added") {
    fail "visible-demand patch was not visible (would park a good close)"
}
# Non-visible demand: non-empty patch with NO token overlap -> park.
if (demand-visible ["retry" "backoff"] "diff --git a/lib/x b/lib/x
+unrelated change") {
    fail "non-visible-demand patch reported visible (fabro-9967 class)"
}
# The fabro-9967 shape itself: empty patch -> always park, with or
# without tokens.
if (demand-visible [] "") { fail "empty patch (no tokens) reported visible" }
if (demand-visible ["closeout"] "") { fail "empty patch (with tokens) reported visible" }
# Degrade: no distinctive tokens + non-empty patch -> visible.
if not (demand-visible [] "+some change") { fail "token-less non-empty patch not visible (degrade broken)" }

print "closeout-smoke: ok — dockerfile-hits + closure-discipline logic verified"

# Sourcing closeout.nu imports its `def main`; nu auto-invokes it after
# the top level runs — exit explicitly so the smoke never reaches it.
exit 0
