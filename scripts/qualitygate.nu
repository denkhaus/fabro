#!/usr/bin/env nu
# Touched-crates quality gate (fabro-5453, world merger): derives the crates
# a run actually changed and gates exactly those — the fabro workspace takes
# 15+ min for a cold full build at the 8-CPU run environment, so the
# develop-workflow tester cannot afford `--workspace` gates (measurement:
# scripts/gate-measure-24cpu-reference.log, 394s cold at 24 CPUs).
# Exit 0 = green. Style follows the retired lab exemplar (archived tag
# scripts/qualitygate.nu): sections, `do { ^cmd } | complete`, first red
# stops the gate.
#
# Base detection reuses the lab evidence.nu pattern: the parent of the LAST
# engine checkpoint commit (subject 'fabro(<run-id>): ...') is the run base —
# works in the shallow sandbox clone without origin refs.

# Flaky-test policy: nextest retries each failing test once. The known
# fabro-80b3 class (for_each_accepts_an_array_at_the_item_limit times out
# under full-suite load) gets its second chance; deterministic failures
# still fail the gate.
const NEXTEST_RETRIES = 1

def current-branch [] {
    (git branch --show-current | str trim)
}

def run-base [] {
    let run_id = (
        current-branch
        | parse --regex 'fabro/run/(?P<id>[^/]+)$'
        | get -o id.0
        | default ''
    )
    if ($run_id | is-empty) {
        # Interactive/human invocation outside a run: diff the working tree
        # against HEAD (uncommitted changes) — the touched-set of a draft.
        return {base: "HEAD", grounded: false}
    }
    let subject_mark = $"fabro\(($run_id)\):"
    let checkpoints = (git log --format=%H --fixed-strings --grep $subject_mark | lines | compact)
    if ($checkpoints | is-empty) {
        return {base: "HEAD", grounded: false}
    }
    let base = (git rev-parse $"($checkpoints | last)^")
    {base: $base, grounded: true}
}

# Map changed paths to workspace crates; root manifest/lock changes mark the
# whole workspace as touched (dependency edits can affect every crate).
def touched-crates [] {
    let base = (run-base).base
    let paths = (git diff --name-only $base | lines | compact)
    let crate_paths = ($paths | where {|p| $p | str starts-with 'lib/' })
    if ($crate_paths | is-empty) {
        return []
    }
    # `parse` yields a table PER input string, so `each` would nest the
    # result (list<list<record>> — the run-1 gate crash). flatten first.
    let crates = ($crate_paths
        | each {|p| $p | parse --regex '^lib/(?:apps|components|foundation)/(?P<crate>[^/]+)/' }
        | flatten
        | get -o crate
        | uniq
        | compact)
    let root_touched = ($paths | where {|p|
        ($p in ['Cargo.toml' 'Cargo.lock' 'rust-toolchain.toml'])
    } | is-not-empty)
    if $root_touched {
        print "root manifest/lock changed -> workspace-wide gate (cargo check)"
        # Sentinel: main routes this to check-workspace-compiles. Returning
        # [] here would silently degrade to fmt-only (run-1 lesson).
        return ["__workspace__"]
    }
    $crates
}

# Toolchain pin (AGENTS.md): rustfmt/clippy results depend on the compiler
# version — the repo pins nightly-2026-04-14, which is also the default (and
# only) toolchain in the toolchain image. Explicit pin keeps the gate
# identical on the host, where the default is stable.
const PINNED_TOOLCHAIN = "nightly-2026-04-14"

def check-fmt [] {
    print "== cargo fmt --check --all =="
    let res = (do { ^cargo $'+($PINNED_TOOLCHAIN)' fmt --check --all } | complete)
    if $res.exit_code != 0 {
        print ($res.stdout | str trim -r -c "\n")
        print ($res.stderr | str trim -r -c "\n")
        return false
    }
    print "format clean"
    true
}

# Loop-asset tier (fabro-bfe1): deterministic machine verification of the
# dev loop's own .nu scripts. Two parts: a side-effect-free parse check of
# every develop-workflow script (`nu --ide-check` parses only — a bare
# `source <file>` would auto-invoke the script's `def main` after import;
# see the closeout-smoke.nu header), then the checked-in evidence-smoke
# regression over evidence.nu's pure helpers. Runs in BOTH main paths —
# crates touched or not — so a loop-asset-only diff is no longer a 4s
# no-op gate (run 01M23TE61D4Y).
def check-loop-assets [] {
    print '== checking loop-asset scripts =='
    # Full-repo nushell tier (lint-nu.nu): parse check of EVERY script —
    # repo scripts/ and all workflow assets, not just develop's — plus the
    # interpolated-regex scan that parse checks cannot see.
    # The gate previously walked develop scripts only; a verify.nu shipped
    # broken through that gap.
    let lint = (do { ^nu scripts/lint-nu.nu } | complete)
    print $lint.stdout
    if ($lint.exit_code != 0) {
        print $lint.stderr
        return false
    }
    # Prompt-lint tier (fabro-41de C3 guard, gate-wired 2026-09-19): loop
    # prompts/graph/toml literals must not rot — unresolvable seed ids,
    # drifted justfile anchors, provenance literals (run ids, PR refs).
    # Runs in BOTH gate paths so prompt-only diffs are gated too.
    let plint = (do { ^nu .fabro/scripts/prompt-lint.nu } | complete)
    print $plint.stdout
    if ($plint.exit_code != 0) {
        print $plint.stderr
        return false
    }
    let smokes = [
        '.fabro/workflows/develop/scripts/evidence-smoke.nu'
        # Claim-gate path battery (fabro-4c81): the pure claim-body-verdict
        # core over canned seeds-show records — blocking classes, creation-
        # intent windows, advisory classes, fail-open contract.
        '.fabro/workflows/develop/scripts/claim-check-smoke.nu'
        # Graph-contract pin (fabro-83df/fabro-92e2, incident 2026-09-19):
        # the develop graph must keep its deterministic-exit contract —
        # planner ungated, preflight report-only, guard exits intact.
        '.fabro/workflows/develop/scripts/graph-contract-smoke.nu'
        # Loop-lane graph-contract pin (fabro-70b5): lane wiring flags,
        # the implementer envelope pin, the red bounce — the meta lane's
        # load-bearing contracts, same tier as the develop pin.
        '.fabro/workflows/loop/scripts/graph-contract-smoke.nu'
        # tracker-guard pure decision logic (fabro-0da8): guard-decision /
        # sd-issue-count over canned complete-style records — both-empty
        # route, open/in_progress arms, and the sd-failure fail-open
        # contract. Found UNREGISTERED by the fabro-8b38 registration
        # sweep (the ac84 silent-de-gate class): the script existed, ran
        # green, and nothing executed it.
        '.fabro/workflows/develop/scripts/tracker-guard-smoke.nu'
        # Salvage-sweep analysis battery (fabro-f312, user directive
        # 2026-09-30): the dump analysis is the sweep's decision core, so
        # the battery pins the five verdict-relevant shapes — failed with
        # real work, journal-only bookkeeping, the green-lie, a diff-less
        # run, and a clean green run — with no live server in reach.
        '.fabro/scripts/salvage-sweep-smoke.nu'
        # closeout pure-decision logic (fabro-5af4/591a era): reviewer
        # journal, deferred-action and exemption-arm sweep. Same finding —
        # unregistered until fabro-8b38.
        '.fabro/workflows/develop/scripts/closeout-smoke.nu'
        # close-claim battery (fabro-2a3b): the close-claim-check core over
        # the incident fixture pair — a subject claiming a close the tracker
        # never recorded is a finding; remainder/residual/file phrasings
        # stay quiet. The LIVE check runs in the session's line watch.
        '.fabro/scripts/close-claim-check-smoke.nu'
        # release-sha battery (fabro-06da): release tags must name the
        # PUSHABLE line tip — only the allow-listed files may derive a short
        # sha, the two release scripts must call the policy site, and the
        # scanner proves it has teeth on a planted fixture.
        '.fabro/scripts/release-sha-fixtures.nu'
    ]
    for smoke in $smokes {
        let res = (do { ^nu $smoke } | complete)
        if ($res.exit_code != 0) {
            print $"loop-asset smoke FAILED: ($smoke)"
            print ($res.stdout | str trim -r -c "\n" | lines | last 20)
            print ($res.stderr | str trim -r -c "\n")
            return false
        }
    }
    # Checked-in fixture batteries (seed fabro-ac84, run 01M2NDGXSKF8YFANJXRFZGC087):
    # the gate must EXECUTE the fixture scripts under .fabro/scripts/, not just
    # parse them. Discovery is explicit and minimal — name each battery; do NOT
    # blanket-run every .fabro/scripts/*.nu (stage-journal.nu and friction-score.nu
    # are tools, not batteries).
    let batteries = [
        '.fabro/scripts/dup-run-check-fixtures.nu'
        # planner-preflight anchor battery (fabro-83df report-only
        # end-to-end case included; 0.5s measured 2026-09-19)
        '.fabro/scripts/planner-preflight-anchor-fixtures.nu'
        # revisor overflow-ledger battery (fabro-552a): fixture revision
        # file with open + consumed overflows drives `open`/`consume`
        # selection deterministically, plus the live real-tree invariant
        # for the memoize entry in revision 01M2X8458MDWMRVBRVEMDX9W4J.
        '.fabro/workflows/revisor/scripts/overflow-ledger-fixtures.nu'
        # run-scope fixtures (fabro-70b5 part D): the run-scope
        # classification RED both ways, both lanes, plus the git base
        # derivation — this tier proves the meta lane's diff boundary.
        '.fabro/scripts/run-scope-fixtures.nu'
        # rust-style-guide skill parity (fabro-6538): .fabro/skills/
        # rust-style-guide (canonical — run-facing, the reviewer
        # contract's source) and .agents/skills/rust-style-guide (the
        # local-session mirror) must stay byte-identical; the battery
        # hash-compares both trees (run-images.nu's sha256 pattern).
        '.fabro/scripts/skill-parity-fixtures.nu'
        # touchpoints parity (fabro-de32, arch-gate sprint 9): every
        # tracked fork-only pin file must be named in the touchpoints
        # registry and every literal row path must exist — the two-pin
        # rule machine-checked in both directions (was reviewer-prompt
        # only; live gaps found at filing: fork_exec_guard x2,
        # fork_structured).
        '.fabro/scripts/touchpoints-parity-fixtures.nu'
    ]
    for battery in $batteries {
        let res = (do { ^nu $battery } | complete)
        if $res.exit_code != 0 {
            print $"loop-asset fixture battery FAILED: ($battery)"
            print ($res.stdout | str trim -r -c "\n" | lines | last 20)
            print ($res.stderr | str trim -r -c "\n")
            return false
        }
    }
    print "loop-asset scripts green"
    true
}

def check-clippy [crates: list<string>] {
    if ($crates | is-empty) { return true }
    # '-p' and the crate name MUST be separate argv elements: a single
    # "-p crate" string gets word-split by nu's external spread, handing
    # cargo a package name with a leading space (run-2 gate crash).
    let pkgs = ($crates | each {|c| ['-p' $c] } | flatten)
    print $"== cargo clippy ($crates | str join ', ') -D warnings =="
    let res = (do { ^cargo $'+($PINNED_TOOLCHAIN)' clippy ...$pkgs --all-targets -- -D warnings } | complete)
    if $res.exit_code != 0 {
        print ($res.stdout | str trim -r -c "\n" | lines | last 30)
        print ($res.stderr | str trim -r -c "\n")
        return false
    }
    print "clippy clean"
    true
}

def check-tests [crates: list<string>] {
    if ($crates | is-empty) { return true }
    let pkgs = ($crates | each {|c| ['-p' $c] } | flatten)
    print $"== cargo nextest ($crates | str join ', ') — retries ($NEXTEST_RETRIES) =="
    let res = (do { ^cargo nextest run ...$pkgs --no-fail-fast --retries $NEXTEST_RETRIES } | complete)
    if $res.exit_code != 0 {
        print ($res.stdout | str trim -r -c "\n" | lines | where {|l| ($l | str contains 'FAIL') or ($l | str contains 'Summary')} | last 20)
        print ($res.stderr | str trim -r -c "\n")
        return false
    }
    print "tests green"
    true
}

# fabro-server's graph-render tests shell out to the `fabro` CLI binary
# (target/debug/fabro), which a touched-crates gate never builds on its own —
# without it the tests skip and real render regressions slip through (seed
# fabro-febd). Explicit dependency: build the renderer bin before the test
# step whenever fabro-server is in the gated set. Stays out of the fmt/clippy
# paths.
def build-renderer-if-needed [crates: list<string>] {
    if not ('fabro-server' in $crates) { return true }
    print '== building fabro CLI renderer binary (fabro-server graph-render tests invoke it) =='
    let res = (do { ^cargo build -p fabro-cli --bin fabro } | complete)
    if $res.exit_code != 0 {
        print ($res.stderr | str trim -r -c "\n" | lines | last 30)
        return false
    }
    print 'renderer binary ready'
    true
}

# Workspace-wide fallback when root manifests changed: a compile check only
# (clippy+tests on all 52 crates would blow the tester timeout).
def check-workspace-compiles [] {
    print '== cargo check --workspace — root manifest changed =='
    let res = (do { ^cargo check --workspace } | complete)
    if $res.exit_code != 0 {
        print ($res.stderr | str trim -r -c "\n" | lines | last 30)
        return false
    }
    print "workspace compiles"
    true
}

# ── Run-scope check (fabro-70b5 part D) ────────────────────────────────
# Diff-side scope enforcement for BOTH lanes. The engine's stage envelope
# (x.fs_write) judges the staged set per node and kills out-of-scope
# writes at checkpoint — but the implementer's shell can stage paths the
# envelope's unset write side never sees (a product implementer has NO
# fs_write pin: any staged file passes the checkpoint guard). This check
# is the deterministic net over the RUN DIFF the graph declares:
#
#   product lane (develop tester): a diff touching loop assets REDs the
#     gate — loop assets are the meta lane's surface, never the product
#     implementer's. Exemptions: .fabro/journal/** (the stage-journal
#     hook's own writes, hook-owned per fabro-b6c5) and
#     .seeds/issues.jsonl + .seeds/config.yaml (tracker bookkeeping:
#     planner claim, close, and the seeds CLI's config rewrite on store
#     ops — fabro-28d8, user decision 2026-10-03).
#   loop lane (loop tester calls `check-run-scope loop`): the diff may
#     touch ONLY the loop assets the loop implementer's fs_write pins
#     (.fabro/**, scripts/**, justfile, .seeds/issues.jsonl,
#     .seeds/config.yaml) — a loop
#     run editing lib/ or docs/ is out of lane and REDs.
#
# Determinism: the CALLING GRAPH fixes the lane (the script line lives in
# the run's workflow spec) — no env sniffing, no run-record lookups.
# Interactive invocations (no run branch) diff the working tree, same as
# touched-crates.

# Pure: one path against the loop-lane allow-list (mirrors the loop
# implementer's x.fs_write — keep the pair in sync; the loop
# graph-contract smoke pins the graph side).
def loop-asset? [p: string]: nothing -> bool {
    # fabro-9973: the ONE lib/ exception. The fabro-dot checked-in-
    # workflows snapshot under this directory is mechanically derived
    # from .fabro/workflows graphs — accepting it (`cargo insta accept`)
    # is part of the loop lane's one-unit graph edit, not a product-code
    # edit. Every other lib/ path stays out of lane.
    if ($p | str starts-with "lib/components/fabro-dot/src/snapshots/") { return true }
    let prefix_hit = ([".fabro/" "scripts/"] | any {|q| $p | str starts-with $q})
    ($prefix_hit) or ($p in ["justfile" ".seeds/issues.jsonl" ".seeds/config.yaml"])
}

# Pure: one path against the product-lane deny-list — EXACTLY develop's
# implementer fs_hide set (.fabro/**, .seeds/**, .agents/**, scripts/**,
# justfile) minus the hook-owned journal and the tracker bookkeeping
# write. `.mulch/**` is deliberately NOT denied: develop's implementer
# has a mandatory lesson-capture contract (`ml record` writes the
# git-tracked expertise files) — refusing it would false-red every
# lesson-recording product run (fabro-70b5 spec review).
def product-denied? [p: string]: nothing -> bool {
    if ($p | str starts-with ".fabro/journal/") { return false }
    if ($p in [".seeds/issues.jsonl" ".seeds/config.yaml"]) { return false }
    let prefix_hit = ([".fabro/" ".agents/" ".seeds/" "scripts/"] | any {|q| $p | str starts-with $q})
    ($prefix_hit) or ($p == "justfile")
}

# Pure: the violating paths for one lane. product -> out-of-lane loop
# assets; loop -> everything outside the loop-asset allow-list.
def scope-violations [paths: list<string>, lane: string]: nothing -> list<string> {
    if $lane == "loop" {
        $paths | where {|p| not (loop-asset? $p)}
    } else {
        $paths | where {|p| product-denied? $p}
    }
}

# The run-diff path list: grounded on the run branch's checkpoint base,
# else the working tree (interactive/manual), same base rule as
# touched-crates.
def run-diff-paths [base: record]: nothing -> list<string> {
    let res = (do { ^git diff --name-only $base.base } | complete)
    if $res.exit_code != 0 { return [] }
    $res.stdout | lines | compact
}

# fabro-dot snapshot tier (fabro-9973): any lane diff that changes a
# checked-in workflow graph (.fabro/workflows/**/*.fabro) must run the
# fabro-dot checked-in-workflows snapshot test BEFORE publish — the
# incident class: a workflow.fabro shape change shipped with an
# unaccepted snapshot (edge counts off) and the dogfood gate paid a
# full compile cycle to catch it. Bounded probe: exactly one crate's
# test, only when the diff actually touches a graph; skipped otherwise.
def check-dot-snapshot [base: record]: nothing -> bool {
    let graphs = (run-diff-paths $base | where {|p|
        ($p | str starts-with '.fabro/workflows/') and ($p | str ends-with '.fabro')
    })
    if ($graphs | is-empty) {
        print '== fabro-dot snapshot skipped — no workflow graph in the diff =='
        return true
    }
    print $"== cargo nextest -p fabro-dot checked_in_workflows — diff touches \(($graphs | length)\) workflow graph\(s\) =="
    let res = (do { ^cargo nextest run -p fabro-dot checked_in_workflows --retries $NEXTEST_RETRIES } | complete)
    if $res.exit_code != 0 {
        print ($res.stdout | str trim -r -c "\n" | lines | where {|l| ($l | str contains 'FAIL') or ($l | str contains 'Summary')} | last 20)
        print ($res.stderr | str trim -r -c "\n" | lines | last 10)
        print 'fabro-dot snapshot drift: a checked-in workflow graph changed shape — run `cargo insta accept` to update lib/components/fabro-dot/src/snapshots/ and ship the .snap update in the SAME diff'
        return false
    }
    print 'fabro-dot snapshot green'
    true
}

# Deterministic run-scope verdict for one lane (product|loop). Prints the
# violations and exits 1 on any; exit 0 when the diff is in lane. An
# empty diff is green (a bookkeeping-only run diff is legitimate).
def check-run-scope [lane: string]: nothing -> nothing {
    if not ($lane in ["product" "loop"]) {
        print -e $"check-run-scope: unknown lane '($lane)' \(product|loop\)"
        exit 2
    }
    let base = (run-base)
    if not $base.grounded {
        print "run-scope base ungrounded: interactive or pre-checkpoint, diffing working tree"
    }
    let violations = (scope-violations (run-diff-paths $base) $lane)
    print $"== run-scope [($lane)] — ($violations | length) violation\(s\) =="
    if ($violations | is-not-empty) {
        for v in $violations { print $"run-scope: ($lane) lane diff touches out-of-scope path: ($v)" }
        exit 1
    }
    print "run-scope green"
}

# Pure: the paths whose exec bit is DROPPED among `git diff --summary`
# lines — exactly the ' mode change 100755 => 100644 <path>' shape git
# emits. The reverse (100644 => 100755, gaining +x) never matches:
# granting the exec bit is legitimate, dropping it through a run diff
# is not (fabro-9569: an exec bit rode a green run twice before anyone
# noticed).
def mode-drop-paths [summary_lines: list<string>]: nothing -> list<string> {
    $summary_lines
    | parse --regex '^ mode change 100755 => 100644 (?<p>.+)$'
    | get -o p
    | default []
}

# Deterministic exec-bit preservation verdict over the run diff
# (fabro-9569): the loop tester's tier behind
# `nu scripts/qualitygate.nu check-mode-preservation`. RED on any
# 100755 => 100644 drop; green on mode-preserving diffs and on +x gains.
# Fail-open on the guard's own errors: a git failure skips the tier
# (lane convention — a broken probe must not RED a sound diff).
def check-mode-preservation []: nothing -> nothing {
    let base = (run-base)
    let res = (do { ^git diff --summary $base.base } | complete)
    if $res.exit_code != 0 {
        print "mode-preservation: git diff --summary failed — skipping (fail-open)"
        return
    }
    let drops = (mode-drop-paths ($res.stdout | lines | compact))
    print $"== mode-preservation — ($drops | length) exec-bit drop\(s\) =="
    if ($drops | is-not-empty) {
        for d in $drops { print $"mode-preservation: exec bit dropped \(100755 => 100644\): ($d)" }
        exit 1
    }
    print "mode-preservation green"
}

# Subcommand surface: `nu scripts/qualitygate.nu check-run-scope <lane>`
# is the loop tester's touched-scope check (and the product lane's manual
# probe). Bare invocation stays the full product gate.
def "main check-run-scope" [lane: string = "product"]: nothing -> nothing {
    check-run-scope $lane
}

# Subcommand surface: `nu scripts/qualitygate.nu check-mode-preservation`
# is the loop tester's exec-bit tier (fabro-9569).
def "main check-mode-preservation" []: nothing -> nothing {
    check-mode-preservation
}

# Subcommand surface: `nu scripts/qualitygate.nu check-dot-snapshot` is
# the loop tester's fabro-dot snapshot tier (fabro-9973) — the same
# check the product gate runs inline, shared instead of copied.
def "main check-dot-snapshot" []: nothing -> nothing {
    let base = (run-base)
    if not $base.grounded {
        print "dot-snapshot base ungrounded: interactive or pre-checkpoint, diffing working tree"
    }
    if not (check-dot-snapshot $base) { exit 1 }
}

def main [] {
    let crates = (touched-crates)
    let base = (run-base)
    if not $base.grounded {
        print 'gate base ungrounded: interactive or pre-checkpoint, diffing working tree'
    }

    # Run-scope tier (fabro-70b5): the product gate refuses loop-asset
    # diffs BEFORE any compile work — a shell-staged .fabro write must
    # not ride a green gate. First red stops (qualitygate style).
    let scope = (scope-violations (run-diff-paths $base) "product")
    if ($scope | is-not-empty) {
        for v in $scope { print $"run-scope: product lane diff touches out-of-scope path: ($v)" }
        print "GATE RED"
        exit 1
    }
    # fabro-dot snapshot tier (fabro-9973): before either branch — a
    # workflow-graph shape change must be snapshot-accepted even in a
    # crates-touched run. First red stops (qualitygate style).
    if not (check-dot-snapshot $base) {
        print "GATE RED"
        exit 1
    }
    if ($crates | is-empty) {
        print "no crates touched"
        let green = ((check-loop-assets) and (check-fmt))
        if $green { print "GATE GREEN"; exit 0 }
        print "GATE RED"
        exit 1
    }
    if ($crates | any {|c| $c == '__workspace__' }) {
        let green = ((check-loop-assets) and (check-fmt) and (check-workspace-compiles))
        if $green { print "GATE GREEN"; exit 0 }
        print "GATE RED"
        exit 1
    }
    print $"touched crates: ($crates | str join ', ')"
    let green = ((check-loop-assets) and (check-fmt) and (check-clippy $crates) and (build-renderer-if-needed $crates) and (check-tests $crates))
    if $green {
        print "GATE GREEN"
        exit 0
    }
    print "GATE RED"
    exit 1
}
