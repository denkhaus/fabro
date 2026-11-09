#!/usr/bin/env nu
# One-shot clone bootstrap (`just bootstrap`): mise itself (installed
# when absent), every tool .mise.toml pins, and the lefthook git hooks.
# Idempotent — safe on every run. Rust/rustup stays OUT of scope: the
# toolchain is owned by rustup (AGENTS.md), and this script arranges
# only what mise owns.
#
# The true cold start (no mise, no just) is documented in
# CONTRIBUTING.md: install mise, then `just bootstrap`.

def main [] {
    # ── mise itself ────────────────────────────────────────────────
    # Absent mise is installed via the official installer; the recipe
    # cannot run without `just` anyway, so this branch serves hosts
    # whose just came from elsewhere (brew, a package manager).
    let mise_bin = (
        if (do { ^mise --version } | complete | get exit_code) == 0 {
            'mise'
        } else {
            print 'bootstrap: mise not found — running the official installer (mise.run)'
            let res = (do { ^sh -c 'curl -fsSL https://mise.run | sh' } | complete)
            if $res.exit_code != 0 {
                print -e $res.stderr
                error make {msg: 'bootstrap: the mise installer failed'}
            }
            print ($res.stdout | str trim)
            let candidate = [$env.HOME '.local' 'bin' 'mise'] | path join
            if not ($candidate | path exists) {
                error make {msg: $"bootstrap: installer finished but ($candidate) is missing — check its output above"}
            }
            $candidate
        }
    )

    # ── the pinned tools ───────────────────────────────────────────
    # trust is idempotent; install --yes never prompts.
    let version = (do { ^$mise_bin --version } | complete | get stdout | str trim)
    print $"bootstrap: ($version)"
    let trust = (do { ^$mise_bin trust } | complete)
    if $trust.exit_code != 0 {
        print -e $trust.stderr
        error make {msg: 'bootstrap: mise trust failed (is this a repo you meant to clone?)'}
    }
    let install = (do { ^$mise_bin install --yes } | complete)
    if $install.exit_code != 0 {
        print -e $install.stderr
        error make {msg: 'bootstrap: mise install failed'}
    }
    print ($install.stdout | lines | where $it =~ 'installed|already' | str join "\n")

    # ── the git hooks ──────────────────────────────────────────────
    let hooks = (do { ^$mise_bin exec -- lefthook install } | complete)
    if $hooks.exit_code != 0 {
        print -e $hooks.stderr
        error make {msg: 'bootstrap: lefthook install failed'}
    }
    print ($hooks.stdout | str trim)

    # ── what is now active ─────────────────────────────────────────
    let pre_commit = ['.git' 'hooks' 'pre-commit'] | path join
    let pre_push = ['.git' 'hooks' 'pre-push'] | path join
    if ($pre_commit | path exists) and ($pre_push | path exists) {
        print 'bootstrap: hooks active (pre-commit, pre-push) — see lefthook.yml'
    } else {
        error make {msg: 'bootstrap: lefthook reported success but the hooks are missing'}
    }
    print 'bootstrap: done. Rust: rustup toolchain install nightly-2026-04-14 --profile minimal --component clippy,rustfmt (AGENTS.md owns the toolchain).'
}
