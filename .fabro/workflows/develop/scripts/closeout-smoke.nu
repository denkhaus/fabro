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

print "closeout-smoke: ok — dockerfile-hits glob/filter logic verified"

# Sourcing closeout.nu imports its `def main`; nu auto-invokes it after
# the top level runs — exit explicitly so the smoke never reaches it.
exit 0
