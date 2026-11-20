#!/usr/bin/env nu
# Deterministic verification dispatcher (fabro-6e7f, user directive
# 2026-09-14): ONE entry point that decides WHICH checks to run from the
# calling agent's workflow STEP plus the current diff — replacing the
# prompt-encoded scope rules that drifted (fabro-0d56, 01M20T9S8).
#
# Stages:
#   implementer — crate-scoped fmt + clippy (default features, -D warnings)
#                 on every code-touched crate; for test-file-touched crates
#                 the new tests BY NAME FILTER when the diff only ADDS test
#                 files, the FULL crate suite when existing tests are edited
#                 (or name extraction comes up empty — fail-safe); a compile
#                 check for code-touched-only crates. Never the gate, never
#                 the workspace suite (the deterministic tester owns both).
#                 Plus the help-snapshot rule (work#1429): when the diff
#                 touches a crate's clap surface (args.rs/main.rs), that
#                 crate's cmd::*::help family joins its nextest selection
#                 — a clap edit that moves help output can no longer pass
#                 verify without executing the snapshots.
#                 Plus the fabro-web lane (work#1304): when the diff touches
#                 apps/fabro-web/** or lib/packages/** (the bun workspace
#                 members), the web surface runs typecheck + tests. Its
#                 cold-start arm (work#1336) installs web deps first when
#                 node_modules is absent — a fresh checkout's first `bun
#                 test` used to die on a missing module until a manual
#                 `bun install`.
# Exit 0 = all checks green. First red stops (qualitygate style:
# `do { ^cmd } | complete`).
#
# Touched-crate + base detection come from the shared verification
# commons (denkhaus/work#1445): the run-id parse, run-base (checkpoint
# hit + the work#1427 journal fallback), crate extraction, test-file /
# inline-cfg(test) classification, web-surface predicate, vendor-surface
# predicate, clap-surface/help-snapshot scope (work#1429), and the
# nextest Summary parser all live ONCE in scripts/lib/verification_
# commons.nu — this script keeps stage dispatch + tier policy only.
# The stage comes from the engine-injected FABRO_STAGE env (fabro-6e7f /
# fork_stage_env.rs) since v2 (fabro-a9cc); an explicit argument remains
# as a manual override.
source lib/verification_commons.nu

const PINNED_TOOLCHAIN = "nightly-2026-04-14"

# fabro-web lane (work#1304 + work#1336): a diff touching apps/fabro-web/**
# or lib/packages/** gets the web surface's typecheck + tests, the Rust
# lanes' tier shape. The repo root is the bun workspace ROOT (root
# bun.lock, bunfig.toml pins the isolated linker, Bun >= 1.3) — the
# cold-start install runs there; app scripts resolve through
# `bun run --cwd apps/fabro-web`.
const WEB_DIR = "apps/fabro-web"
const WEB_INSTALL_TIMEOUT_SECS = 600

# Bounded cold-start install (work#1336): `timeout` bounds a hung install;
# when the binary is absent (minimal image) the install runs unbounded —
# a missing guard is better than a broken lane.
def web-install-bounded [] {
    if (which timeout | is-empty) {
        do { ^bun install --frozen-lockfile } | complete
    } else {
        do { ^timeout $WEB_INSTALL_TIMEOUT_SECS bun install --frozen-lockfile } | complete
    }
}

# fabro-f750: FAIL paths print a bounded TAIL of the captured output.
# Cargo emits the actual diagnostics AFTER the warning preamble, so a
# head-bound can push the error past the cut (run 01M4KZ2BCS4C: an E0004
# sat behind a 2k-char warning preamble on a FAIL clippy verdict).
def output-tail [text: string, bound: int] {
    let trimmed = ($text | str trim)
    let length = ($trimmed | str length)
    if $length <= $bound {
        $trimmed
    } else {
        $trimmed | str substring ($length - $bound)..
    }
}


# Test-file / inline-cfg(test) classification (is-test-file,
# first-cfg-line, is-inline-test-diff) lives in the shared verification
# commons module — sourced above (denkhaus/work#1445).

# Crates touched by the diff, split into {code: [...], tests: [...]}:
# `tests` = crates with at least one test-file change, `code` = every
# touched crate (tests included — fmt/clippy cover them too).
# `help_scope` (work#1429): per-crate help-snapshot test names for every
# crate whose diff touches the clap surface (args.rs/main.rs). The rule
# lives ONCE in the module (crate-help-snapshot-scope); this composition
# only carries the result — a crate without a help-snapshot family drops
# out (no-op, never a zero-match filter).
def touched [] {
    let run = (run-base '' --origin-branch --checkpoint-pick 'latest')
    let base = $run.base
    # work#1482: the module's enumeration joins untracked files — the
    # bare git diff missed brand-new files, and their crates never ran.
    let paths = (diff-paths $base)
    let crate_paths = ($paths | where {|p| $p | str starts-with 'lib/' })
    let crates = (crates-from-paths $crate_paths)
    let test_files = ($crate_paths
        | where {|p| (is-test-file $p) or (is-inline-test-diff $base $p) })
    let test_crates = (crates-from-paths $test_files)
    # The clap surface (work#1429): the owning crate's cmd::*::help
    # family joins its nextest selection.
    let help_scope = ($paths
        | where {|p| clap-surface-path? $p}
        | each {|p| crate-of-path $p}
        | uniq
        | each {|c| ({crate: $c, names: (crate-help-snapshot-scope $paths $c)}) }
        | where {|r| ($r.names | is-not-empty)})
    # The web surface (work#1304): the bun workspace members, via the
    # shared predicate. lib/packages counts — fabro-api-client is
    # workspace:*-consumed by the web app, so its diffs verify through
    # the web suite too.
    let web = ($paths | any {|p| web-surface-path? $p})
    {base: $base, how: $run.how, code: $crates, tests: $test_crates, test_files: $test_files, help_scope: $help_scope, web: $web}
}

# Test function names declared under a test attribute in a file's Rust
# source: the attribute line (#[test], #[tokio::test(...)], #[e2e_test...],
# ...) sets a pending flag; the immediately following `fn <name>` captures
# the name. Extraction-only heuristic — an empty result never narrows
# verification (the caller falls back to the full crate suite).
def test-names [content: string] {
    mut names = []
    mut pending = false
    for l in ($content | lines) {
        let t = ($l | str trim)
        if ($t | str starts-with '#[') {
            if ($t | str contains 'test') { $pending = true }
        } else if $pending {
            let m = ($t | parse --regex '^(?:pub\s+)?(?:const\s+)?(?:async\s+)?fn\s+(?P<name>[A-Za-z0-9_]+)')
            if ($m | is-not-empty) {
                $names = ($names | append $m.name.0)
                $pending = false
            } else if ($t | str starts-with 'fn') or ($t | str starts-with 'pub fn') {
                $pending = false
            }
        }
    }
    $names
}

# The nextest Summary parser lives in the shared verification commons
# (nextest-summary / nextest-filter-verdict, with the work#1424/#1432
# FAIL-vs-tool-error classification) — sourced above.

# Per-crate nextest scope (fabro-7f58): when EVERY test-touched file in the
# crate is purely added (absent from the base, present in the worktree),
# run only the extracted test names; any file that edits existing tests
# (present in base — modified or deleted) widens to the full crate suite.
# Fail-safe on both edges: no extractable names and unclassifiable files
# both widen to the full suite.
def test-mode-for [base: string, files: list<string>] {
    mut added = []
    for p in $files {
        let old = (do { git show $"($base):($p)" } | complete)
        if $old.exit_code == 0 {
            return {mode: 'full', reason: $"($p) edits existing tests"}
        }
        if (not ($p | path exists)) {
            return {mode: 'full', reason: $"($p) deleted"}
        }
        $added = ($added | append $p)
    }
    mut names = []
    for p in $added {
        $names = ($names | append (test-names (open --raw $p)))
    }
    let flat = ($names | flatten | uniq | compact)
    if ($flat | is-empty) { return {mode: 'full', reason: 'no test names extracted'} }
    {mode: 'names', names: $flat}
}

# This crate's entry from the work#1429 help-snapshot scope ([] when the
# crate has none — e.g. a clap-surface diff in a crate without a
# tests/it/cmd/ family). The flatten matters: `get` on a table is
# column-oriented, so a one-row lookup yields list<list<string>> — the
# nextest spread and the expected count need the flat name list.
def help-names-for [help_scope: table, crate: string] {
    $help_scope | where {|h| $h.crate == $crate} | get -o names | default [] | flatten
}

# The by-name nextest tier, shared by the added-tests slice (fabro-7f58)
# and the work#1429 help-snapshot slices: verdict-classified (work#1424/
# work#1432), FAIL exits 1, tool-error exits 3 — never a silent green.
def nextest-names-tier [crate: string, names: list<string>, label: string] {
    print $"== nextest -p ($crate) \(($label): ($names | str join ', '))=="
    let t0 = (date now)
    let res = (do { ^cargo nextest run -p $crate --no-fail-fast --retries 2 ...$names } | complete)
    let secs = (((date now) - $t0) / 1sec | math round)
    # Zero-selection guard (fabro-bdff), classified by the shared
    # verdict (work#1424/work#1432). The summary travels on STDERR —
    # the verdict parses the COMBINED streams. Two different reds:
    #   FAIL (exit 1)      — a real test verdict: non-zero exit, or
    #                      the filter matched fewer tests than
    #                      extracted ("0 tests matched the filter"
    #                      included);
    #   TOOL-ERROR (exit 3) — the display has no Summary line at
    #                      all: the tool broke; NOT a gate verdict.
    let verdict = (nextest-filter-verdict $res.exit_code ($res.stdout + $res.stderr) ($names | length))
    if $verdict.verdict == 'tool-error' {
        print (output-tail ($res.stdout + $res.stderr) 3000)
        print "verify: TOOL-ERROR nextest — output unparseable (no Summary line in stdout+stderr): tooling failure, not a gate verdict (work#1424/#1432)"
        $env.LAST_VERIFY_FAIL = "nextest-tool-error"
        exit 3
    }
    if $verdict.verdict == 'fail' {
        print (output-tail ($res.stdout + $res.stderr) 3000)
        print $"verify: FAIL nextest ($crate) \(name filter: ($verdict.reason)\)"
        $env.LAST_VERIFY_FAIL = "nextest"
        exit 1
    }
    print $"verify: PASS nextest ($crate) \(($secs)s\)"
}

# The TS lane itself (work#1304): cold-start install when node_modules is
# absent, then typecheck and the package test script (`bun run test` =
# `bun test --parallel`). First red exits — the Rust lanes' FAIL shape
# (captured-output tail, tier label, exit 1), never a `return`.
def stage-web [] {
    if (not ($"($WEB_DIR)/node_modules" | path exists)) {
        print $"verify: no node_modules in ($WEB_DIR) — installing web deps \(bun install --frozen-lockfile, bounded at ($WEB_INSTALL_TIMEOUT_SECS)s\) ..."
        let t0 = (date now)
        let res = (web-install-bounded)
        let dur = ((date now) - $t0)
        if $res.exit_code != 0 {
            print (output-tail ($res.stdout + $res.stderr) 3000)
            print "verify: FAIL bun-install fabro-web"
            exit 1
        }
        print $"verify: PASS bun-install fabro-web \(install took ($dur)\)"
    }
    print $"== bun run typecheck — ($WEB_DIR) =="
    let res = (do { ^bun run --cwd $WEB_DIR typecheck } | complete)
    if $res.exit_code != 0 {
        print (output-tail ($res.stdout + $res.stderr) 3000)
        print "verify: FAIL typecheck fabro-web"
        exit 1
    }
    print $"verify: PASS typecheck fabro-web"
    print $"== bun run test — ($WEB_DIR) =="
    let res = (do { ^bun run --cwd $WEB_DIR test } | complete)
    if $res.exit_code != 0 {
        print (output-tail ($res.stdout + $res.stderr) 3000)
        print "verify: FAIL test fabro-web"
        exit 1
    }
    print $"verify: PASS test fabro-web"
}

def stage-implementer [] {
    let t = (touched)
    if (($t.code | is-empty) and (not $t.web)) {
        print "verify: no lib/ crates and no web surface touched — nothing to verify (parse-level checks belong to the brief)"
        return
    }
    # Base observability (work#1427): say WHICH fallback arm grounded the
    # scope — a recovery pass must be visible in the surviving output.
    if $t.how == 'journal' {
        print "verify: checkpoint miss on the run branch — base is the last journal commit's parent (work#1427 fallback)"
    } else if $t.how == 'head-parent' {
        print "verify: checkpoint and journal detection both missed — last-resort HEAD^ base"
    }
    print $"verify: code-touched=($t.code | str join ', ') test-file-touched=($t.tests | str join ', ')"
    if ($t.help_scope | is-not-empty) {
        # work#1429 observability: WHICH crates gained the help-snapshot
        # scope and how many of their cmd::*::help tests ride the lane.
        print $"verify: clap-surface diff \(args.rs/main.rs, work#1429\) — help-snapshot tests in scope for: ($t.help_scope | get crate | str join ', ')"
    }
    if $t.web {
        print $"verify: web surface touched \(($WEB_DIR), lib/packages\)"
    }
    # A FAIL path must EXIT, never `return`: `return` leaves the def and
    # main ends green — a red tier reads as exit 0 for every caller (the
    # just recipe, the run gate). Observed live 2026-11-01: three
    # `verify: FAIL nextest fabro-cli` runs, exit 0 each.
    for c in $t.code {
        print $"== fmt -p ($c) =="
        let res = (do { ^cargo $'+($PINNED_TOOLCHAIN)' fmt -p $c --check } | complete)
        if $res.exit_code != 0 {
            print $res.stderr
            print $"verify: FAIL fmt ($c)"
            $env.LAST_VERIFY_FAIL = "fmt"
            exit 1
        }
        print $"verify: PASS fmt ($c)"
    }
    # Per-crate progress with elapsed time (work#1410): a tool-ceiling
    # timeout (600s) kills verify mid-phase, and the captured output tail
    # is all a rerun sees — one duration-bearing line per completed phase
    # per crate keeps the surviving crate list (and each slice's cost)
    # visible instead of discarding it with the dead process.
    for c in $t.code {
        let label = ("== clippy -p " + $c + " (default features, deny warnings) ==")
        print $label
        let t0 = (date now)
        let res = (do { ^cargo $'+($PINNED_TOOLCHAIN)' clippy -p $c --all-targets -- -D warnings } | complete)
        let secs = (((date now) - $t0) / 1sec | math round)
        if $res.exit_code != 0 {
            print (output-tail $res.stderr 2000)
            print $"verify: FAIL clippy ($c)"
            exit 1
        }
        print $"verify: PASS clippy ($c) \(($secs)s\)"
    }
    let code_only = ($t.code | where {|c| $c not-in $t.tests })
    for c in $code_only {
        print $"== compile check -p ($c) =="
        let res = (do { ^cargo check -p $c --quiet } | complete)
        if $res.exit_code != 0 {
            print (output-tail $res.stderr 2000)
            print $"verify: FAIL compile ($c)"
            exit 1
        }
        print $"verify: PASS compile ($c)"
        # work#1429: a code-only clap-surface diff used to end HERE — the
        # help snapshots never executed, and a stale help snapshot landed
        # silently (the work#1361 finding). The owning crate's
        # cmd::*::help family joins the lane instead.
        let help_names = (help-names-for $t.help_scope $c)
        if ($help_names | is-not-empty) {
            nextest-names-tier $c $help_names 'help-snapshot tests, clap-surface diff (work#1429)'
        }
    }
    for c in $t.tests {
        let files = ($t.test_files | where {|p| (crate-of-path $p) == $c })
        let tm = (test-mode-for $t.base $files)
        if $tm.mode == 'names' {
            nextest-names-tier $c $tm.names 'added tests by name filter'
            # work#1429: the clap surface moved in a crate whose diff also
            # ADDS test files — the help-snapshot family runs as its own
            # slice (separate filters, so the added-tests verdict keeps
            # its extracted-name accounting). A full-suite crate already
            # covers the family, so it gains nothing here.
            let help_names = (help-names-for $t.help_scope $c)
            if ($help_names | is-not-empty) {
                nextest-names-tier $c $help_names 'help-snapshot tests, clap-surface diff (work#1429)'
            }
        } else {
            print $"== nextest -p ($c) — full crate suite: ($tm.reason) =="
            # --retries 2: a load-bound host (a sibling build, a bench)
            # must not kill a sound tier — the qualitygate's discipline,
            # applied to the same command here.
            let t0 = (date now)
            let res = (do { ^cargo nextest run -p $c --no-fail-fast --retries 2 } | complete)
            let secs = (((date now) - $t0) / 1sec | math round)
            if $res.exit_code != 0 {
                print (output-tail ($res.stdout + $res.stderr) 3000)
                print $"verify: FAIL nextest ($c)"
                exit 1
            }
            print $"verify: PASS nextest ($c) \(($secs)s\)"
        }
    }
    # The web lane (work#1304) runs AFTER the Rust lanes, same first-red-
    # stops discipline — a mixed diff verifies both surfaces in one call.
    if $t.web {
        stage-web
    }
    print "verify: implementer stage green"
}

# Stage source (fabro-a9cc, v2): the engine-injected FABRO_STAGE env is
# authoritative inside a run (fork_stage_env.rs, fabro-6e7f); an explicit
# argument overrides it for local/manual use only.
def main [stage?: string] {
    let stage = (if ($stage == null or $stage == '') { null } else { $stage })
    let resolved = (
        if $stage != null {
            let env_stage = ($env | get -o FABRO_STAGE | default '')
            if ($env_stage != '') and ($env_stage != $stage) {
                print $"verify: WARN argument '($stage)' overrides injected FABRO_STAGE '($env_stage)' — manual use only"
            }
            $stage
        } else if (($env | get -o FABRO_STAGE | default '') != '') {
            $env.FABRO_STAGE
        } else {
            print 'verify: no stage — pass one (e.g. `just verify implementer`) or run inside a stage (FABRO_STAGE injected)'
            exit 2
        }
    )
    let src = (if $stage != null { 'argument' } else { 'FABRO_STAGE' })
    print $"verify: stage ($resolved) from ($src)"
    match $resolved {
        'implementer' => { stage-implementer }
        _ => {
            print $"verify: unknown stage '($resolved)' — v2 supports implementer"
            exit 2
        }
    }
}
