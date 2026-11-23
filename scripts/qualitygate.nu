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
# Base detection comes from the shared verification commons
# (denkhaus/work#1445): the parent of the LAST engine checkpoint commit
# (subject 'fabro(<run-id>): ...') is the run base — works in the shallow
# sandbox clone without origin refs. The checkpoint-miss arm carries the
# work#1427 journal fallback (base = the last journal commit's parent),
# healing this gate's former HEAD-on-miss drift by construction.

# Shared creation-intent window (fabro-48f3): anchor_check.nu is the ONE
# seam — claim-check and planner-preflight source the same file, so both
# gates judge the same spec body the same way (no local regex copy).
# `source` resolves against this file's directory.
source ../.fabro/workflows/develop/scripts/anchor_check.nu

# The verification commons (denkhaus/work#1445): the run-id parse, run-base
# (checkpoint pick 'earliest' — this gate's whole-run scope, now with the
# work#1427 checkpoint-miss journal fallback healed here too), the crate
# extraction, and the web/vendor surface predicates live ONCE in
# scripts/lib/verification_commons.nu. No local copy of those shapes
# remains (the regrowth pin in scripts/verification-commons-fixtures.nu
# holds that).
source lib/verification_commons.nu

# Flaky-test policy: nextest retries each failing test once. The known
# fabro-80b3 class (for_each_accepts_an_array_at_the_item_limit times out
# under full-suite load) gets its second chance; deterministic failures
# still fail the gate.
const NEXTEST_RETRIES = 1

# Map changed paths to workspace crates; root manifest/lock changes mark the
# whole workspace as touched (dependency edits can affect every crate).
def touched-crates [] {
    let base = (run-base).base
    let paths = (git diff --name-only $base | lines | compact)
    let crate_paths = ($paths | where {|p| $p | str starts-with 'lib/' })
    if ($crate_paths | is-empty) {
        return []
    }
    let crates = (crates-from-paths $crate_paths)
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

# The engine injects FABRO_SERVER=/run/fabro/channel.sock into every shell
# a run spawns (ADR-0035); the CLI's env-attuned --server default turns it
# into args.target.server = Some and breaks env-attuned parse tests plus
# any tool reading the env without its own scrub (run 01M621RW4YPD's gate
# analysis, work#1054 — verified on the untouched parent 38c801372). The
# gate's CARGO tiers run with the var scrubbed; the channel calls (work
# facade tiers) keep it. One seam, both lanes' gates.
def cargo-env [block: closure]: nothing -> any {
    with-env {FABRO_SERVER: null} { do $block }
}

# work#1610: `--all` scopes fmt to the workspace members; vendor/ stays
# out ONLY while root Cargo.toml keeps the vendored dirs in [workspace]
# exclude. The exclusion is asserted explicitly — a members/exclude edit
# that would silently pull vendor into fmt scope reds here instead of
# rustfmt-ing the snapshots (the run-01M7YV5Y churn class).
def vendor-fmt-exclusion-guard []: nothing -> bool {
    let manifest = (open Cargo.toml)
    let excluded = ($manifest | get -o workspace.exclude | default [])
    let missing = (['vendor/petri' 'vendor/pebble' 'vendor/sandbox-driver'] | where {|d| $d not-in $excluded})
    if ($missing | is-not-empty) {
        print $"vendor fmt exclusion RED: root Cargo.toml [workspace].exclude no longer names every vendored dir \(missing: ($missing | str join ', ')\) — cargo fmt --all would reach the vendored snapshots"
        return false
    }
    print "vendor fmt exclusion: all vendored dirs stay workspace-excluded"
    true
}

def check-fmt [] {
    if not (vendor-fmt-exclusion-guard) { return false }
    print "== cargo fmt --check --all =="
    let res = (cargo-env { do { ^cargo $'+($PINNED_TOOLCHAIN)' fmt --check --all } | complete })
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
# see the reporter-smoke.nu header), then the checked-in evidence-smoke
# regression over evidence.nu's pure helpers. Runs in BOTH main paths —
# crates touched or not — so a loop-asset-only diff is no longer a 4s
# no-op gate (run 01M23TE61D4Y).
def check-loop-assets [] {
    print '== checking loop-asset scripts =='
    # One battery-runner surface (fabro-eae5): the smokes/batteries
    # registry lives in .fabro/scripts/battery-runner.nu and only
    # there — this gate calls the runner (`all` scope); the loop
    # lane's tester gate that shared it retired with the loop
    # workflow (ADR-0039 stage G). Loop-asset tiers (fabro-bfe1 lint,
    # fabro-41de prompt-lint, fabro-ac84 fixture batteries) run in
    # BOTH main paths — crates touched or not — so a loop-asset-only
    # diff is no longer a 4s no-op gate (run 01M23TE61D4Y).
    let res = (do { ^nu .fabro/scripts/battery-runner.nu all } | complete)
    print $res.stdout
    if ($res.exit_code != 0) {
        print $res.stderr
        return false
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
    let res = (cargo-env { do { ^cargo $'+($PINNED_TOOLCHAIN)' clippy ...$pkgs --all-targets -- -D warnings } | complete })
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
    let res = (cargo-env { do { ^cargo nextest run ...$pkgs --no-fail-fast --retries $NEXTEST_RETRIES } | complete })
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
    let res = (cargo-env { do { ^cargo build -p fabro-cli --bin fabro } | complete })
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
    let res = (cargo-env { do { ^cargo check --workspace } | complete })
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
#     hook's own writes, hook-owned per fabro-b6c5). The .seeds tracker
#     paths left these lists with the fs-envelope sweep (denkhaus/work#990).
#   loop lane (loop tester calls `check-run-scope loop`): the diff may
#     touch ONLY the loop assets the loop implementer's fs_write pins
#     (.fabro/**, scripts/**, justfile) — a loop
#     run editing lib/ or docs/ is out of lane and REDs.
#
# Determinism: the CALLING GRAPH fixes the lane (the script line lives in
# the run's workflow spec) — no env sniffing, no run-record lookups.
# Interactive invocations (no run branch) diff the working tree, same as
# touched-crates.

# Pure: one path against the loop-lane allow-list — moved to the
# shared lib below (the claim gate emits the same extraction into
# stage output for the implementer's pre-tester self-check), sourced
# here so every qualitygate caller and fixture keeps the names.
source ../.fabro/scripts/lib/spec_paths.nu

# Pure: one path against the product-lane deny-list — EXACTLY develop's
# implementer fs_hide set (.agents/**, ADR-0049: loop assets are product
# surfaces now). `.mulch/**` is deliberately NOT denied: develop's implementer
# has a mandatory lesson-capture contract (`mulch record` writes the
# git-tracked expertise files) — refusing it would false-red every
# lesson-recording product run (fabro-70b5 spec review).
def product-denied? [p: string]: nothing -> bool {
    # ADR-0049: loop assets are product surfaces — the only denied path
    # is the .agents/ mirror (owned by `just sync-style-guide`; the
    # implementer's fs_hide list is the same single entry).
    $p | str starts-with ".agents/"
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
    let res = (cargo-env { do { ^cargo nextest run -p fabro-dot checked_in_workflows --retries $NEXTEST_RETRIES } | complete })
    if $res.exit_code != 0 {
        print ($res.stdout | str trim -r -c "\n" | lines | where {|l| ($l | str contains 'FAIL') or ($l | str contains 'Summary')} | last 20)
        print ($res.stderr | str trim -r -c "\n" | lines | last 10)
        print 'fabro-dot snapshot drift: a checked-in workflow graph changed shape — run `cargo insta accept` to update lib/components/fabro-dot/src/snapshots/ and ship the .snap update in the SAME diff'
        return false
    }
    print 'fabro-dot snapshot green'
    true
}

# ── Web tier (work#1304 + work#1336) ───────────────────────────────────
# The verify.nu web lane's gate twin: a diff touching apps/fabro-web/** or
# lib/packages/** (the bun workspace members) runs the web surface's
# typecheck + tests. Cold-start arm: the install runs first when
# node_modules is absent, bounded and with a progress line — a fresh
# checkout's first `bun test` used to die on a missing module until a
# manual `bun install`. A Rust-only diff skips the tier in one path check
# — the gate stays fast for lib/-only changes. Install runs at the
# workspace ROOT (root bun.lock, bunfig.toml pins the isolated linker,
# Bun >= 1.3); app scripts resolve through `bun run --cwd apps/fabro-web`.
const WEB_DIR = "apps/fabro-web"
const WEB_INSTALL_TIMEOUT_SECS = 600

def web-diff? [base: record]: nothing -> bool {
    run-diff-paths $base | any {|p| web-surface-path? $p}
}

def check-web [base: record]: nothing -> bool {
    if (not (web-diff? $base)) {
        print '== web tier skipped — no fabro-web/lib-packages files in the diff =='
        return true
    }
    print $"== bun web tier — typecheck + tests \(($WEB_DIR)\) =="
    if (not ($"($WEB_DIR)/node_modules" | path exists)) {
        print $"web tier: no node_modules in ($WEB_DIR) — installing \(bun install --frozen-lockfile, bounded at ($WEB_INSTALL_TIMEOUT_SECS)s\)"
        let res = (if (which timeout | is-empty) {
            do { ^bun install --frozen-lockfile } | complete
        } else {
            do { ^timeout $WEB_INSTALL_TIMEOUT_SECS bun install --frozen-lockfile } | complete
        })
        if $res.exit_code != 0 {
            print ($res.stdout | str trim -r -c "\n" | lines | last 30)
            print ($res.stderr | str trim -r -c "\n" | lines | last 30)
            return false
        }
        print 'web deps installed'
    }
    let res = (do { ^bun run --cwd $WEB_DIR typecheck } | complete)
    if $res.exit_code != 0 {
        print ($res.stdout | str trim -r -c "\n" | lines | last 30)
        print ($res.stderr | str trim -r -c "\n" | lines | last 30)
        return false
    }
    print 'typecheck clean'
    let res = (do { ^bun run --cwd $WEB_DIR test } | complete)
    if $res.exit_code != 0 {
        print ($res.stdout | str trim -r -c "\n" | lines | last 30)
        print ($res.stderr | str trim -r -c "\n" | lines | last 30)
        return false
    }
    print 'web tests green'
    true
}

# ── Vendor tier (denkhaus/work#1444) ───────────────────────────────────
# `just vendor-pins` as a RECORDED gate arm: a diff touching vendor/**
# (the vendored fork snapshots or vendor/PINS.toml) runs the ADR-0034
# provenance verification inside the gate transcript — previously the
# content-hash/tree-sha check rode only inside the loop-asset battery's
# anonymous battery list, so a green vendor diff rested on the evidence
# header alone (run 01M7QEQT1P8CF3SF4D4W8SN7BE, work#1444). A red verdict
# fails the gate; no vendor path in the diff skips the tier in one path
# check — the web tier's idiom.
def vendor-diff? [base: record]: nothing -> bool {
    run-diff-paths $base | any {|p| vendor-surface-path? $p}
}

# ── Vendor whitespace tier (denkhaus/work#1610) ────────────────────────
# Fail fast BEFORE any expensive tier when the run diff's vendor/ story
# is formatting churn only: every changed vendored snapshot file differs
# from the base by whitespace alone (the rustfmt-over-vendor incident,
# run 01M7YV5Y — two gate bounces). The rule it names: vendored snapshots
# are exempt from formatting drift by design — the repo's fmt/clippy
# tiers never reach vendor/, and neither may a run; content changes must
# ride `just vendor export`. A content-changed vendor diff does NOT trip
# here — the vendor-pins tier judges provenance. Cheap by construction:
# git show + strip-compare, no cargo.
def check-vendor-whitespace [base: record]: nothing -> bool {
    let paths = (run-diff-paths $base)
    if (not ($paths | any {|p| vendor-surface-path? $p})) {
        print '== vendor whitespace tier skipped — no vendor/ files in the diff =='
        return true
    }
    let split = (vendor-churn-split $base.base $paths)
    if ($split.content | is-not-empty) {
        print $"== vendor whitespace tier — ($split.content | length) content-changed vendor file\(s\), provenance rides the vendor-pins tier =="
        return true
    }
    if ($split.churn | is-empty) {
        print '== vendor whitespace tier skipped — only PINS.toml changed in vendor/ =='
        return true
    }
    print $"== vendor whitespace RED — ($split.churn | length) vendored file\(s\) changed whitespace-only \(formatting churn\):"
    for p in $split.churn { print $"  ($p)" }
    print 'vendored snapshots are exempt from formatting drift by design — the repo fmt/clippy tiers never touch vendor/, and neither may a run'
    print 'content changes must ride `just vendor export`; revert the churn (git checkout -- <path>) or redo the change through the fork'
    false
}

def check-vendor-pins [base: record]: nothing -> bool {
    if (not (vendor-diff? $base)) {
        print '== vendor tier skipped — no vendor/ files in the diff =='
        return true
    }
    print '== just vendor-pins — diff touches vendor/ (denkhaus/work#1444) =='
    let res = (do { ^just vendor-pins } | complete)
    if $res.exit_code != 0 {
        print ($res.stdout | str trim -r -c "\n" | lines | last 30)
        print ($res.stderr | str trim -r -c "\n" | lines | last 10)
        print 'vendor pins RED: the vendored trees no longer match vendor/PINS.toml — reconcile first (just vendor status / just vendor import); a vendor diff never ships past a red provenance check'
        return false
    }
    print ($res.stdout | str trim -r -c "\n")
    print 'vendor pins green'
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

# ── Seed-spec path tier (fabro-8a60) ───────────────────────────────────
# The loop-asset tier's diff-side twin of claim-check's citation gate:
# every path the run's claimed seed spec NAMES under the loop-asset
# roots must appear in the run diff — an empty-edit or wrong-file pass
# REDs at the tester in seconds instead of consuming a full review
# cycle. Extraction is deliberately conservative (no false REDs):
#   - only slash-tokens count: a bare `justfile` in prose is a word,
#     not a path citation;
#   - URL payloads are stripped first (same rule as claim-check) so a
#     `https://host/x` token cannot parse as a path;
#   - glob tokens (`.fabro/**`) and bare directory prefixes
#     (`scripts/`) are dropped — the diff lists files, a glob or
#     directory prefix can never match one;
#   - a bare DIRECTORY token (`.fabro/workflows`, no trailing slash)
#     drops too, resolved against the worktree: the prefix rule only
#     caught the `scripts/` spelling, and a file-only
#     `git diff --name-only` can never contain a directory — keeping
#     it was a PERMANENT false RED (fabro-48f3: the d83a pre-claim
#     body rewrite, run 01M5844);
#   - creation-named paths drop (fabro-48f3): a path the body names
#     inside an add/create-naming window is a proposed target, not an
#     edit-this-file mandate — the claim gate's shared seam;
#   - the loop-asset? allow-list decides "under the loop-asset roots"
#     — the SAME predicate run-scope uses, one list, no drift;
#   - `.fabro/journal/` paths drop: a journal citation in a spec body
#     is PROVENANCE (the Origin/Basis lines point at the reviewer run
#     that filed the residual), never an edit mandate — the journal is
#     append-only stage records no pass edits, so keeping the citation
#     was a permanent false RED on every residual seed (fabro-6b0f).
# The extraction itself is the shared `spec-named-loop-paths` sourced
# above from the lib — one list, no drift.

# Pure: the named paths the diff is missing (the RED set). Empty diff +
# named paths -> every path missing -> RED; that is the tier's point.
export def spec-paths-missing [named: list<string>, diff: list<string>]: nothing -> list<string> {
    $named | where {|p| $p not-in $diff}
}

# Pure: the item this run holds a live claim on, from a
# `fabro work claims --project fabro --json` payload — the claimable
# resolution core (exported for the fixture battery). A run holds at most
# one live claim per project (ADR-0032); zero or many is ambiguity the
# gate cannot attribute.
export def claimed-item-for-run [claims: list<any>, run_id: string]: nothing -> string {
    let mine = ($claims
        | where {|c| (($c | get -o runId | default '') == $run_id)}
        | get -o itemKey
        | default [])
    if (($mine | length) == 1) { $mine | first } else { '' }
}

# Deterministic seed-spec path verdict: resolve the run's claimed work
# item (explicit --seed wins — the manual probe arm; otherwise the run's
# live claim on the tracker, zero/many is ambiguity the gate cannot
# attribute), extract the spec-named loop-asset paths from the item
# body, and RED on any named path the run diff does not touch. Fail-open
# on the tier's own tooling errors (work CLI failure, JSON surprise,
# unresolvable item) — a broken probe must not RED a sound diff (lane
# convention). NOTE: legacy-id --seed values resolve once the server
# seam ships with fork.26; until then pass full keys.
def "main check-seed-spec-paths" [--seed: string = ""]: nothing -> nothing {
    let base = (run-base)
    if not $base.grounded {
        print "seed-spec-paths base ungrounded: interactive or pre-checkpoint, diffing working tree"
    }
    let diff = (run-diff-paths $base)
    let run_id = (id-from-branch (current-branch))
    let seed = (if ($seed | is-not-empty) {
        $seed
    } else {
        let list = (do { ^fabro work --server https://mirtuell.net claims --project fabro --json } | complete)
        if $list.exit_code != 0 {
            print "seed-spec-paths: work claims failed — skipping (fail-open)"
            ""
        } else {
            let claims = (try { $list.stdout | from json } catch { [] })
            let claimed = (claimed-item-for-run $claims $run_id)
            if ($claimed | is-empty) {
                print "seed-spec-paths: no live claim for this run — skipping (fail-open)"
                ""
            } else {
                $claimed
            }
        }
    })
    if ($seed | is-empty) { return }
    let show = (do { ^fabro work --server https://mirtuell.net show --project fabro $seed --json } | complete)
    if $show.exit_code != 0 {
        print $"seed-spec-paths: work show ($seed) failed — skipping \(fail-open\)"
        return
    }
    let desc = (try { $show.stdout | from json | get -o body | default '' } catch { '' })
    let named = (spec-named-loop-paths $desc)
    if ($named | is-empty) {
        print $"== seed-spec-paths [($seed)] — spec names no loop-asset paths, tier idle =="
        return
    }
    let missing = (spec-paths-missing $named $diff)
    print $"== seed-spec-paths [($seed)] — ($named | length) loop-asset path\(s\) named, ($missing | length) missing =="
    for n in $named {
        let where = (if ($n in $diff) { 'in diff' } else { 'NOT in diff' })
        print $"seed-spec-paths: named ($n) — ($where)"
    }
    if ($missing | is-not-empty) {
        print "seed-spec-paths RED: the seed spec names loop-asset paths the run diff does not touch — an empty-edit or wrong-file pass"
        exit 1
    }
    print "seed-spec-paths green"
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

# Subcommand surface: `nu scripts/qualitygate.nu check-vendor-pins` is
# the vendor tier's direct probe (denkhaus/work#1444) — the same check
# the product gate runs inline, shared instead of copied; also the drive
# arm for the close-when evidence over a vendor/ scratch change.
def "main check-vendor-pins" []: nothing -> nothing {
    let base = (run-base)
    if not $base.grounded {
        print "vendor-pins base ungrounded: interactive or pre-checkpoint, diffing working tree"
    }
    if not (check-vendor-pins $base) { exit 1 }
}

# Subcommand surface: `nu scripts/qualitygate.nu check-vendor-whitespace`
# is the whitespace tier's direct probe (denkhaus/work#1610) — the same
# fail-fast the product gate runs inline.
def "main check-vendor-whitespace" []: nothing -> nothing {
    let base = (run-base)
    if not $base.grounded {
        print "vendor-whitespace base ungrounded: interactive or pre-checkpoint, diffing working tree"
    }
    if not (check-vendor-whitespace $base) { exit 1 }
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
    # Vendor whitespace tier (denkhaus/work#1610): first of the vendor
    # checks and ahead of EVERY expensive tier — a whitespace-only vendor
    # diff is formatting churn the snapshots are exempt from by design;
    # name the rule and stop here. First red stops (qualitygate style).
    if not (check-vendor-whitespace $base) {
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
    # Web tier (work#1304): before the crates branches — a fabro-web diff
    # must run the TS checks even when no lib/ crate is touched. First red
    # stops (qualitygate style).
    if not (check-web $base) {
        print "GATE RED"
        exit 1
    }
    # Vendor tier (denkhaus/work#1444): before the crates branches — a
    # vendor/ diff must show the PINS provenance verification green in
    # this transcript even when no lib/ crate is touched. First red
    # stops (qualitygate style).
    if not (check-vendor-pins $base) {
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
