#!/usr/bin/env nu
# Deterministic quality gate. Exit 0 = green, non-zero = red.
# Called by `just qualitygate`; the develop workflow's tester step goes
# through the same target, so there is exactly one gate definition.
# Each check prints its own section and returns false (with details) on
# failure; main stops at the first red check.

const LARGE_FILE_LIMIT = 1mb

# ---------------------------------------------------------------------------
# checks — nothing -> bool (false = red, details already printed)
# ---------------------------------------------------------------------------

def check-sync []: nothing -> bool {
    print "== workflow sync (product <-> meta) =="
    let res = (do { nu .fabro/workflows/develop/scripts/sync-check.nu } | complete)
    print ($res.stdout | str trim -r -c "\n")
    if $res.exit_code != 0 {
        print ($res.stderr | str trim -r -c "\n")
        return false
    }
    true
}

def check-scripts []: nothing -> bool {
    print "== nu-check (all nu scripts) =="
    # ADR-0006 (meta): every nu script in this world's tree must parse —
    # the product gate and the workflow both execute these scripts.
    let scripts = [(glob .fabro/workflows/*/scripts/*.nu) (glob scripts/*.nu)] | flatten
    if ($scripts | is-empty) {
        print "no nu scripts found"
        return false
    }
    let broken = ($scripts | each {|s|
        if (try { nu-check $s } catch { false }) { null } else { {script: $s} }
    } | compact)
    if ($broken | is-not-empty) {
        $broken | each {|b| print $"script failed nu-check: ($b.script)" }
        return false
    }
    print $"syntax-clean ($scripts | length) scripts"
    true
}

def check-large-files []: nothing -> bool {
    print "== tracked large files =="
    let big = (
        git ls-files
        | lines
        | par-each {|f| {file: $f, size: (ls $f | get 0.size)}}
        | where size > $LARGE_FILE_LIMIT
    )
    if ($big | length) > 0 {
        print "build artifacts or large binaries are tracked — untrack them (.gitignore + git rm --cached):"
        $big | each {|it| print $"($it.size) \t ($it.file)"}
        return false
    }
    true
}

def check-gofmt []: nothing -> bool {
    print "== gofmt check =="
    let unformatted = (gofmt -l . | lines | compact)
    if ($unformatted | length) > 0 {
        print "unformatted files:"
        print ($unformatted | str join "\n")
        return false
    }
    true
}

def check-go [phase: string]: nothing -> bool {
    print $"== go ($phase) =="
    let res = (do { ^go $phase ./... } | complete)
    if $res.exit_code != 0 {
        print $res.stdout
        print $res.stderr
        return false
    }
    print ($res.stdout | str trim -r -c "\n")
    true
}

# Build into a temp dir OUTSIDE the worktree: a bare `go build ./...` drops
# the compiled binary into the repo root on every gate run (artifact hygiene).
def check-go-build []: nothing -> bool {
    print "== go build =="
    let out = (mktemp --directory)
    let res = (do { go build -o $"($out)/" ./... } | complete)
    rm -rf $out
    if $res.exit_code != 0 {
        print $res.stdout
        print $res.stderr
        return false
    }
    true
}

# a9bb step 2 — deterministic run-scope rule: run diffs must not touch
# the workflow assets (.fabro/workflows/). Platform work happens via
# platform-namespace PRs, never run merge-backs. Journal
# (.fabro/journal/) and tracker (.seeds/) writes are legitimate
# run-branch content and deliberately stay out of the rule.
def current-branch []: nothing -> string {
    git branch --show-current | str trim
}

def check-run-scope []: nothing -> bool {
    print "== run scope (workflow assets untouchable from runs) =="
    let branch = (current-branch)
    if not ($branch | str starts-with "fabro/run/") {
        print $"skip: '($branch)' is not a run branch"
        return true
    }
    # The world the run was cut from. Run workspaces carry the local
    # world branch plus its origin ref (verified in a real run sandbox);
    # prefer the local branch — it names exactly the commit the run
    # started from.
    let bases = (
        ["denkhaus-lab" "origin/denkhaus-lab"]
        | each {|ref| do -i { git merge-base HEAD $ref } | default '' | str trim }
        | compact
    )
    if ($bases | is-empty) {
        print "skip: no world-branch merge base resolvable in this workspace"
        return true
    }
    let base = ($bases | first)
    # base -> working tree (staged + unstaged) is the real run diff;
    # untracked files need git status (git diff cannot see them).
    let touched = (
        [(git diff --name-only $base -- .fabro/workflows/ | lines | compact)
         (git status --porcelain -- .fabro/workflows/ | lines | compact | each {|l| $l | str substring 3..})]
        | flatten
        | uniq
    )
    if ($touched | is-not-empty) {
        print "run diff touches workflow assets — platform work belongs in platform-namespace PRs (a9bb step 2):"
        $touched | each {|p| print $"  ($p)"}
        return false
    }
    let short = ($base | str substring 0..7)
    print $"no workflow assets in run diff — base ($short)"
    true
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

def main []: nothing -> nothing {
    if not (check-sync) { exit 1 }
    if not (check-run-scope) { exit 1 }
    if not (check-scripts) { exit 1 }
    if not (check-large-files) { exit 1 }
    if not (check-gofmt) { exit 1 }
    if not (check-go-build) { exit 1 }
    if not (check-go vet) { exit 1 }
    if not (check-go test) { exit 1 }
    print "== qualitygate passed =="
}
