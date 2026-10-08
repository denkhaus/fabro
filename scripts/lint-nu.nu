#!/usr/bin/env nu
# lint-nu: side-effect-free lint for every nushell script the repo ships.
#
# Two layers, both born from real incidents:
#   1. parse  — `nu --ide-check` reports Error-severity diagnostics
#      (unbalanced delimiters, bad syntax). Parse-only by design: a bare
#      `source <file>` would auto-invoke the script's `def main`.
#   2. interpolated-regex scan — an interpolated string (`$'...'`/`$"..."`)
#      evaluates EVERY `(...)` as a subexpression, so a regex group like
#      `(?P<sha>...)` parses clean and dies at RUNTIME with
#      `Command ?P<sha>... not found` (scripts/verify.nu, run
#      01M2GVW7GGGB's implementer). `nu --ide-check` and `nu-check` both
#      miss this class — the scan is the only reliable detector.
#
# Scope: repo scripts/ plus every workflow's loop assets. The qualitygate
# (fabro-bfe1) previously walked ONLY develop scripts — this is the
# full-repo replacement, also wired into `just validate-workflows`.

def script-paths [] {
    (glob scripts/*.nu)
    | append (glob .fabro/workflows/*/scripts/*.nu)
    | append (glob scripts/**/*.nu)
    | append (glob .fabro/scripts/*.nu)
    | uniq
    | sort
}

def parse-check [file: string] {
    # --ide-check exits 0 even on parse errors: diagnostics are JSON lines
    # on stdout; an Error-severity line is the failure signal.
    let res = (do { ^nu --ide-check 10 $file } | complete)
    let errors = ($res.stdout | lines | where {|l| $l | str contains '"severity":"Error"' })
    if ($errors | is-not-empty) {
        print $"parse FAILED: ($file)"
        print ($errors | last 5)
        false
    } else {
        true
    }
}

def interpolated-regex-check [file: string] {
    # A `$'`/`$"` string containing a regex group opener `(?P<`, `(?<`, or
    # `(?:` is always the subexpression bug — plain strings or
    # concatenation are the fix.
    # Comment lines are skipped: prose about the pattern (this file's own
    # docblock) is not an occurrence.
    let text = (open --raw $file)
    let hits = ($text | lines | enumerate | where {|it|
        ($it.item | str trim | str starts-with '#') == false and (
            # Backtick raw string: regex quotes and backslashes stay literal.
            $it.item | parse --regex `\$['"][^'"\n]*\(\?(?:P<|<|:)` | is-not-empty
        )
    })
    if ($hits | is-not-empty) {
        for h in $hits {
            print $"interpolated-regex FAILED: ($file):($h.index + 1) — regex group inside an interpolated string"
            print $"  ($h.item | str trim)"
        }
        false
    } else {
        true
    }
}

# Bare-paren-text check: inside an interpolated string ($'...' / $"...")
# EVERY (...) is a subexpression — literal prose parens like
# (fabro-1a41, guards investigation) PARSE as list syntax and explode at
# RUNTIME with 'Command ... not found' (two crashes on 2026-09-28: the
# workbench seed-ensure success print and the known-red yellow print; nu
# --ide-check cannot catch this class). A paren group containing a comma is
# list syntax and never a useful interpolation — flag it.
def paren-comma-bare [line: string] {
    # examine ONLY the interpolated-string spans themselves ($'...' / $"..."):
    # record literals on the same line carry legitimate commas in braces, and
    # `str join ', '` carries its comma inside quotes inside a legitimate
    # subexpression — neither is bare text-paren list syntax
    let spans = ($line | parse --regex `(?P<s>\$["'][^"']*["'])`)
    if ($spans | is-empty) { false } else {
        # escaped parens \(...\) are the CORRECT literal-paren idiom — neutralize them first
        ($spans | get s | any {|s| ($s | str replace -ar `\\[()]` 'X' | parse --regex `\([^()]*,[^()]*\)` | is-not-empty) })
    }
}

def bare-paren-check [file: string] {
    let text = (open --raw $file)
    let hits = ($text | lines | enumerate | where {|it|
        (($it.item | str trim | str starts-with '#') == false) and (($it.item | str contains '$"') and (paren-comma-bare $it.item))
    })
    if ($hits | is-not-empty) {
        for h in $hits {
            print $"bare-paren FAILED: ($file):($h.index + 1) — comma paren group inside an interpolated string is list syntax, not text"
            print $"  ($h.item | str trim)"
        }
        false
    } else {
        true
    }
}

# Registration check (fabro-8b38): the gates execute smokes and fixture
# batteries ONLY when they are named in the battery runner's explicit
# lists (.fabro/scripts/battery-runner.nu — the ONE registry since
# fabro-eae5, called by both scripts/qualitygate.nu and the loop lane's
# loop-gate.nu), so a new battery that nobody registers is silently
# un-gated — the class that produced fabro-ac84 and, caught by this
# check's first run, tracker-guard-smoke.nu + closeout-smoke.nu (both
# existed, both ran green, nothing executed them). Convention:
# *-smoke.nu belongs in the `smokes` list, *-fixtures.nu in
# `batteries`. Scripts that match a suffix but are not batteries go in
# the allow-list below with a reason.
const REGISTRATION_ALLOW = []

def registration-check [] {
    let registry = '.fabro/scripts/battery-runner.nu'
    if not (($registry | path exists)) {
        print $"registration FAILED: ($registry) is missing — the battery registry moved?"
        return false
    }
    let text = (open --raw $registry)
    let candidates = (
        script-paths
        | where {|f|
            let base = ($f | path basename)
            (($base | str ends-with '-smoke.nu') or ($base | str ends-with '-fixtures.nu')) and ($f not-in $REGISTRATION_ALLOW)
        }
    )
    # script-paths mixes absolute (glob without a ./ prefix) and relative
    # entries; normalize to repo-root-relative before matching qualitygate's
    # literal list entries.
    let root = $env.PWD
    let unregistered = (
        $candidates
        | where {|f| not ($text | str contains ($f | path relative-to $root)) }
    )
    if ($unregistered | is-empty) {
        print $"registration ok: ($candidates | length) smoke/fixture batteries all named in ($registry)"
        true
    } else {
        for f in $unregistered {
            let list = (if (($f | path basename) | str ends-with '-smoke.nu') { 'smokes' } else { 'batteries' })
            print $"registration FAILED: ($f | path relative-to ($env.PWD)) is not named in ($registry) — add it to the ($list) list"
        }
        false
    }
}

# NUL-byte check (fabro-40ba): a literal NUL in a script makes git treat the
# file as BINARY - no diff, no review, and `grep` prints 'binary file
# matches' instead of lines, so the rg-first reconnaissance this repo
# prescribes silently degrades on it. One battery carried a NUL for months
# (its fixture meant to write control bytes; the fix is an escape sequence:
# "\u{0}" in the source, never the raw byte).
def nul-byte-check [file: string] {
    # bytes index-of is the only reliable reader here: `each` over a binary
    # yields binary chunks and `into int` refuses more than 8 bytes, so a
    # byte-wise loop cannot see the NUL at all.
    let bytes = (open --raw $file | into binary)
    let at = ($bytes | bytes index-of 0x[00])
    if $at >= 0 {
        print $"nul-byte FAILED: ($file | path relative-to ($env.PWD)) contains a NUL byte at offset ($at) — write it as an escape sequence, never literally"
        false
    } else {
        true
    }
}

def main [] {
    let scripts = (script-paths)
    if ($scripts | is-empty) {
        print 'lint-nu: no scripts found — scope broken'
        exit 2
    }
    print $"lint-nu: ($scripts | length) scripts"
    mut green = true
    for s in $scripts {
        if not (parse-check $s) { $green = false }
        if not (interpolated-regex-check $s) { $green = false }
        if not (bare-paren-check $s) { $green = false }
        if not (nul-byte-check $s) { $green = false }
    }
    if not (registration-check) { $green = false }
    if $green {
        print 'lint-nu: green'
    } else {
        print 'lint-nu: FAILED'
        exit 1
    }
}
