#!/usr/bin/env nu
# lint-engine-coupling: the engine must not name a workflow of the fabro
# project (ADR-0052, incident denkhaus/work#1716). Fabro is a
# multi-project tool: what a workflow needs from the engine is a
# declaration (workflow.toml / project.toml), never a slug table in
# engine code. This lint makes that rule mechanical. It derives the
# forbidden names at run time from the directories under
# .fabro/workflows/ (no hardcoded list) and scans the engine (every
# *.rs under lib/, every *.rs / *.ts / *.tsx under apps/) for a quoted
# string literal that IS such a name ("name" in Rust; "name" or 'name'
# in TypeScript). Backticks are not quotes on purpose: prose that names
# a workflow in a comment must use backticks and stays unflagged. Test
# code is exempt. A hit is red unless scripts/lint-engine-coupling-
# exceptions.txt lists its exact `<path> <quoted-literal> — <reason>`
# line; an exception that matches no current hit is itself a failure,
# so the list can only shrink — and must reach empty.

# Directory segments whose files are generated, third-party or test code.
const EXCLUDED_DIR_SEGMENTS = ['node_modules' 'dist' 'target' '.vercel' 'tests' 'test_support']
# Generated clients, build output and the test-support crate living inside the
# scan roots.
const EXCLUDED_PATH_PREFIXES = ['lib/packages/fabro-api-client/src/' 'lib/foundation/fabro-test/']
const EXCEPTIONS_FILE = 'scripts/lint-engine-coupling-exceptions.txt'


# The forbidden names: this project's workflow directories, nothing else.
def workflow-names [] {
    let dirs = (ls .fabro/workflows | where type == dir | get name | path basename | sort)
    if ($dirs | is-empty) {
        print 'lint-engine-coupling: no workflow directories under .fabro/workflows — scope broken'
        exit 2
    }
    $dirs
}

def is-test-file [path: string] {
    let name = ($path | path basename)
    ($name in ['tests.rs' 'test_support.rs']) or ($name ends-with '_tests.rs') or ($name ends-with '_test.rs') or ($name ends-with '.test.ts') or ($name ends-with '.test.tsx')
}

def is-excluded [path: string] {
    let segments = ($path | split row '/')
    let excluded = ($segments | any {|s| $s in $EXCLUDED_DIR_SEGMENTS })
    $excluded or ($EXCLUDED_PATH_PREFIXES | any {|p| $path starts-with $p })
}

def engine-files [] {
    # glob returns absolute paths; report and except on repo-relative ones.
    let root = (pwd)
    let rust = (glob 'lib/**/*.rs' | each {|p| $p | path relative-to $root } | where {|p| not (is-excluded $p) and not (is-test-file $p) })
    let web = (
        (glob 'apps/**/*.ts' | append (glob 'apps/**/*.tsx'))
        | each {|p| $p | path relative-to $root }
        | where {|p| not ($p ends-with '.d.ts') and not (is-excluded $p) and not (is-test-file $p) }
    )
    $rust | append $web | uniq | sort
}

# Test code is exempt: an inline test module (from a line starting with
# `#[cfg(test)]` through its closing brace, by brace depth) is not scanned. A
# test module can sit mid-file with production code after it (e.g.
# handler/work_items.rs), so only the module is dropped, not the rest of
# the file; line numbers stay those of the real file.
def production-lines [text: string] {
    mut kept = []
    mut depth = 0
    for it in ($text | split row "\n" | enumerate) {
        let t = ($it.item | str trim)
        if $depth > 0 {
            $depth = ($depth + ($t | split row "{" | length) - 1) - (($t | split row "}" | length) - 1)
        } else if ($t starts-with '#[cfg(test)]') {
            $depth = 1
        } else {
            $kept = ($kept | append $it.index)
        }
    }
    $kept
}

# The quoted literals a hit can take in this file: double quotes
# everywhere, single quotes additionally in TypeScript.
def needles-for [path: string, name: string] {
    let double = ('"' + $name + '"')
    if ($path ends-with '.ts') or ($path ends-with '.tsx') {
        [$double ("''" + $name + "''")]
    } else {
        [$double]
    }
}

# One record per line and literal that carries a forbidden name. The
# whole-text probe keeps files without any needle off the line walk.
def scan-file [path: string, names: list<string>] {
    let text = (open --raw $path)
    let needles = ($names | each {|name| needles-for $path $name } | flatten | where {|needle| $text | str contains $needle })
    mut hits = []
    if ($needles | is-empty) { return $hits }
    let lines = ($text | split row "\n")
    for i in (production-lines $text) {
        let line = ($lines | get $i)
        for needle in ($needles | where {|needle| $line | str contains $needle }) {
            $hits = ($hits | append {path: $path, line: ($i + 1), literal: $needle})
        }
    }
    $hits
}

def load-exceptions [] {
    if not ($EXCEPTIONS_FILE | path exists) {
        print $'lint-engine-coupling: ($EXCEPTIONS_FILE) missing — scope broken'
        exit 2
    }
    let rows = (open --raw $EXCEPTIONS_FILE | split row "\n")
    let entries = ($rows | enumerate | where {|it|
        let t = ($it.item | str trim)
        ($t | is-not-empty) and (not ($t starts-with '#'))
    })
    $entries | each {|it|
        let parsed = ($it.item | parse --regex '^(?P<path>\S+) (?P<literal>"[^"]*"|''[^'']*'') — (?P<reason>.+)$')
        if ($parsed | is-empty) {
            print $'lint-engine-coupling: ($EXCEPTIONS_FILE):($it.index + 1) is malformed — expected `<path> <quoted-literal> — <reason>`'
            exit 2
        }
        $parsed.0 | insert line ($it.index + 1)
    }
}

# Lint the engine for project-workflow names (ADR-0052).
# Exit 0: every hit excepted, no stale exception. Exit 1: an unexcepted
# hit or a stale exception. Exit 2: broken scope. Run from the repo root.
def main [] {
    let names = (workflow-names)
    let files = (engine-files)
    if ($files | is-empty) {
        print 'lint-engine-coupling: no engine files under lib/ and apps/ — scope broken'
        exit 2
    }
    let hits = ($files | reduce --fold [] {|f, acc| $acc | append (scan-file $f $names) })
    let exceptions = (load-exceptions)

    let forbidden = ($hits | where {|h| not ($exceptions | any {|e| $e.path == $h.path and $e.literal == $h.literal }) })
    let excepted = ($hits | length) - ($forbidden | length)
    let stale = ($exceptions | where {|e| not ($hits | any {|h| $e.path == $h.path and $e.literal == $h.literal }) })

    for h in ($hits | each {|h| $h | insert reason ($exceptions | where {|e| $e.path == $h.path and $e.literal == $h.literal } | get -o 0.reason) }) {
        let tag = if $h.reason == null { 'FORBIDDEN' } else { $'excepted: ($h.reason)' }
        print $'($h.path):($h.line): ($h.literal) [($tag)]'
    }
    for e in $stale {
        print $'($EXCEPTIONS_FILE):($e.line): stale exception ($e.path) ($e.literal) — matches no current hit; delete the line'
    }
    let verdict = if (($forbidden | is-empty) and ($stale | is-empty)) { 'green' } else { 'FAILED' }
    let n_hits = ($hits | length)
    let n_forbidden = ($forbidden | length)
    let n_stale = ($stale | length)
    let summary = $"lint-engine-coupling: ($names | length) workflow names, ($files | length) engine files, ($n_hits) hits: ($excepted) excepted, ($n_forbidden) forbidden, ($n_stale) stale exceptions — ($verdict)"
    print $summary
    if $verdict == 'FAILED' { exit 1 }
}
