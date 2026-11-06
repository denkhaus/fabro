#!/usr/bin/env nu
# Smoke test for reporter.nu — the facts-only stage (ADR-0039 stage G,
# denkhaus/work#1107/#1115): the pure fact machinery (Dockerfile
# detection, demand tokens, visibility verdicts, fact heads) plus the
# full-script path through the channel shim (packet validation, the two
# facts, ZERO tracker mutations — no close, no filings). The closeout's
# park gate, sweeps, and close retired with its authority; their arms
# retired with them. The `source` const resolves against THIS file's
# directory, so the script runs from any cwd:
#   nu .fabro/workflows/develop/scripts/reporter-smoke.nu

const REPORTER = "reporter.nu"
const SCRIPTS_DIR = (path self | path dirname)
source $REPORTER

# The shared smoke harness (denkhaus/work#1109): the fail-log marker
# gate, the shim env fixtures, the run-shaped repo builder, and the
# real-parser argv pin driver live in .fabro/scripts/lib/ — this smoke
# is a consumer, never a re-implementation.
use ../../../scripts/lib/smoke_harness.nu [
    start-fail-log, fail, end-fail-gate, fresh-work-state, run-shaped-repo, drive,
    refuses-missing-server, pin-argvs, tree-bin
]

# nu 0.115 catches `exit` inside `try` (the e2e arms live in one), so a
# bare exit-1 fail would print FAIL and still end green — the battery
# reads exit codes. The harness's fail gate handles it: every fail lands
# in the marker file, the closing gate turns any recorded fail into a
# real non-zero exit. The top-level load-env persists (a def's env never
# reaches the caller — the harness docs name the mechanic).
load-env {SMOKE_FAIL_LOG: (start-fail-log 'reporter-smoke')}

# Positives: every Dockerfile* segment shape the detection must catch.
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

# Empty input degrades to empty — no fact.
if (dockerfile-hits []) != [] { fail "empty input not empty" }

# --- The visibility check (the gate retired; the check feeds the fact) --
# Token extraction: distinctive >=4-char tokens survive; function-word
# stopwords, short tokens, and duplicates drop.
let toks = (demand-tokens "Reporter visibility check: an item closes only when its demand is visible in the run diff, else a painpoint is mandatory")
for expected in ["reporter" "demand" "visible" "diff"] {
    if not ($toks | any {|t| $t == $expected }) { fail $"demand-tokens dropped '($expected)'" }
}
for banned in [seed when else only this with from] {
    if ($toks | any {|t| $t == $banned }) { fail $"demand-tokens kept stopword '($banned)'" }
}
if ($toks | any {|t| ($t | str length) < 4 }) { fail "demand-tokens kept a <4-char token" }

# Visible demand: matching token in a non-empty patch.
if not (demand-visible ["reporter" "retry"] "diff --git a/.fabro/x b/.fabro/x
+reporter fact added") {
    fail "visible-demand patch was not visible"
}
# Non-visible demand: non-empty patch with NO token overlap.
if (demand-visible ["retry" "backoff"] "diff --git a/lib/x b/lib/x
+unrelated change") {
    fail "non-visible-demand patch reported visible (fabro-9967 class)"
}
# The fabro-9967 shape itself: empty patch -> never visible.
if (demand-visible [] "") { fail "empty patch (no tokens) reported visible" }
if (demand-visible ["reporter"] "") { fail "empty patch (with tokens) reported visible" }
# Degrade: no distinctive tokens + non-empty patch -> visible.
if not (demand-visible [] "+some change") { fail "token-less non-empty patch not visible (degrade broken)" }

# --- Reporter facts (ADR-0039 stage C, denkhaus/work#1107) --------------
# The trigger-based category rule (grilled): a category encodes whether a
# fact needs a DECISION, never which decision — painpoints force the
# adjudicator's agent pass, notes never trigger. Visibility NOT visible
# is the fabro-02c4 class (as a note the guard would erode); visible is
# context. A DEGRADED check (tooling failure) is a painpoint too. A
# Dockerfile touch always has an action pending (the image rebuild,
# fabro-6f6e).
let vf_note = (visibility-fact "visible" "denkhaus/work#1" "abc1234")
if $vf_note.category != "note" {
    fail $"visible demand must be a note: ($vf_note | to json -r)"
}
if not ($vf_note.text | str contains "abc1234") {
    fail $"the visibility note must name the diff base: ($vf_note.text)"
}
# The machine head (ADR-0039 stage D, grilled): line one is deterministic
# `key=value` pairs — the retry-dedupe key and the fast path's parse.
if ($vf_note.text | lines | first) != "fact=patch-visibility verdict=visible base=abc1234 item=denkhaus/work#1" {
    fail $"the visible fact's head must be deterministic: ($vf_note.text | lines | first)"
}
if ($vf_note.text | lines | length) < 2 {
    fail "the head carries prose after line one"
}
let vf_painpoint = (visibility-fact "not-visible" "denkhaus/work#1" "abc1234")
if $vf_painpoint.category != "painpoint" {
    fail $"NOT-visible demand must be a painpoint (fabro-02c4): ($vf_painpoint | to json -r)"
}
if not ($vf_painpoint.text | str contains "NOT visible") {
    fail $"the visibility painpoint must say what the diff shows: ($vf_painpoint.text)"
}
if ($vf_painpoint.text | lines | first) != "fact=patch-visibility verdict=not-visible base=abc1234 item=denkhaus/work#1" {
    fail $"the not-visible fact's head must be deterministic: ($vf_painpoint.text | lines | first)"
}
let vf_degraded = (visibility-fact "degraded" "denkhaus/work#1" "abc1234")
if $vf_degraded.category != "painpoint" {
    fail $"a degraded check must be a painpoint: ($vf_degraded | to json -r)"
}
if not ($vf_degraded.text | str contains "not be determined") {
    fail $"the degraded fact must say the verdict was never established: ($vf_degraded.text)"
}
if ($vf_degraded.text | lines | first) != "fact=patch-visibility verdict=degraded base=abc1234 item=denkhaus/work#1" {
    fail $"the degraded fact's head must be deterministic: ($vf_degraded.text | lines | first)"
}
if $vf_note.text == $vf_painpoint.text {
    fail "the two visibility verdicts must read differently"
}

# The Dockerfile fact exists only on hits (no empty-entry noise): a hit
# is a painpoint naming the paths and the inert-until-rebuild reason.
if (dockerfile-fact []) != null {
    fail "no Dockerfile touch must record no fact"
}
let df_hit = (dockerfile-fact [".fabro/Dockerfile.toolchain" "Dockerfile"])
if $df_hit.category != "painpoint" {
    fail $"a Dockerfile touch must be a painpoint: ($df_hit | to json -r)"
}
if not ($df_hit.text | str contains ".fabro/Dockerfile.toolchain") {
    fail $"the Dockerfile fact must name the paths: ($df_hit.text)"
}
if not ($df_hit.text | str contains "INERT") {
    fail $"the Dockerfile fact must name the inert-until-rebuild reason: ($df_hit.text)"
}
if ($df_hit.text | lines | first) != "fact=dockerfile-touch paths=.fabro/Dockerfile.toolchain,Dockerfile" {
    fail $"the Dockerfile fact's head must be deterministic: ($df_hit.text | lines | first)"
}

# The shared parser (lib/fact_head.nu): heads parse to records, records
# render back to the canonical line, prose-only text parses null — the
# engine-retry dedupe keys on the head line (the grilled decision).
use ../../../scripts/lib/fact_head.nu [parse-head, head-of]
let parsed = (parse-head $vf_painpoint.text)
if $parsed == null {
    fail "the parser must read the visibility head"
}
if ($parsed | get -o fact) != "patch-visibility" { fail $"the parser reads fact: ($parsed | to json -r)" }
if ($parsed | get -o verdict) != "not-visible" { fail $"the parser reads verdict: ($parsed | to json -r)" }
if ($parsed | get -o base) != "abc1234" { fail $"the parser reads base: ($parsed | to json -r)" }
if ($parsed | get -o item) != "denkhaus/work#1" { fail $"the parser reads item: ($parsed | to json -r)" }
if (head-of $parsed) != ($vf_painpoint.text | lines | first) {
    fail $"the renderer round-trips: (head-of $parsed)"
}
if (parse-head "plain prose, no head at all") != null {
    fail "prose-only text parses to null"
}
if (parse-head $"($vf_note.text | lines | first)\nmore prose") == null {
    fail "the head parses with prose after it"
}


# --- The reporter through the channel shim (facts only, zero writes) ----
# The full-script path IS the pin: a temp git repo (deterministic diff
# for the visibility check), a shim state file — and the subprocess run
# proves the packet validation, the facts, and the RETIRED authority:
# no close call, no filings, no tracker mutation beyond the fact
# records and the title show. Positive AND negative arms, per the
# edited-script dry-run rule. The shim path resolves ABSOLUTE from the
# smoke's own location — the subprocess runs in the temp repo, a
# relative path would not exist there.

# Temp git repo shaped like a RUN (the harness builder): the diff
# carries the title tokens deterministically for the visibility check.
const RUN_ID = "01SMOKE0000000000000000000"
let orig_cwd = (pwd)
let repo = (run-shaped-repo $RUN_ID "frobnicator queue bounded: the deterministic implementing change")
try {
    cd $repo

    # (a) Visible-demand arm: title tokens present in the diff -> the
    # visibility NOTE records, exit 0, ZERO tracker mutations (the
    # title show and the fact record only), the item stays open.
    let happy_state = ($repo | path join 'happy-state.json')
    fresh-work-state $happy_state "Make the frobnicator queue bounded"
    let happy_run = (drive ($SCRIPTS_DIR | path join "reporter.nu") $happy_state [--work-item] '' '{"itemKey": "denkhaus/work#1"}')
    if $happy_run.exit_code != 0 {
        fail $"visible-demand arm failed: ($happy_run.stderr | str substring 0..300)"
    }
    if not ($happy_run.stdout | str contains "reported facts for denkhaus/work#1") {
        fail $"the reporter must name the item on exit: ($happy_run.stdout | str substring 0..300)"
    }
    let happy_after = (open --raw $happy_state | from json)
    if (($happy_after.log | where {|e| $e.argv.1 not-in ["show" "record"]} | length) != 0) {
        fail "the reporter must never touch the tracker beyond the title show and the fact records"
    }
    if (($happy_after.log | where {|e| $e.argv.1 == "close"}) | length) != 0 {
        fail "the close authority is RETIRED — a close call is a stage-G regression"
    }
    if (($happy_after.items | where state == "open") | length) != 1 {
        fail "the reporter must leave the item open (the chain closes)"
    }
    let happy_facts = ($happy_after.log | where {|e| $e.argv.0 == "observation"})
    if ($happy_facts | length) != 1 {
        fail $"the visible-demand arm records exactly the visibility note: ($happy_facts | to json -r)"
    }
    if $happy_facts.0.flags.category != "note" {
        fail $"a visible demand records as a note: ($happy_facts.0.flags | to json -r)"
    }

    # (b) Not-visible arm: title tokens absent from the diff -> the
    # visibility PAINPOINT records (the fabro-02c4 class), exit 0 (the
    # run reports facts and ends — the CHAIN parks, never the stage),
    # no close, item open.
    let park_state = ($repo | path join 'not-visible-state.json')
    fresh-work-state $park_state "Rewrite the quantum flux capacitor harness"
    let nv_run = (drive ($SCRIPTS_DIR | path join "reporter.nu") $park_state [--work-item] '' '{"itemKey": "denkhaus/work#1"}')
    if $nv_run.exit_code != 0 {
        fail $"not-visible arm must exit 0 \(facts, not a gate\): ($nv_run.stderr | str substring 0..200)"
    }
    let nv_after = (open --raw $park_state | from json)
    let nv_facts = ($nv_after.log | where {|e| $e.argv.0 == "observation"})
    if ($nv_facts | length) != 1 {
        fail $"the not-visible arm records exactly the visibility painpoint: ($nv_facts | to json -r)"
    }
    if $nv_facts.0.flags.category != "painpoint" {
        fail $"a not-visible demand records as a painpoint (fabro-02c4): ($nv_facts.0.flags | to json -r)"
    }
    if not ($nv_facts.0.flags.text | str contains "NOT visible") {
        fail $"the not-visible fact says what the diff shows: ($nv_facts.0.flags.text)"
    }
    if (($nv_after.items | where state == "open") | length) != 1 {
        fail "the not-visible arm must leave the item open"
    }

    # (d) Fact-failure discipline (the spec's stage-C decision 4, pinned
    # discriminating): a FAILED observation record FAILS THE STAGE. With
    # the run identity bound (FABRO_RUN_ID) and a dead server, the title
    # show degrades (advisory) but the FACT call fails on transport —
    # the run must exit 1 naming the failed record. The absorbing
    # mutation (print + return) exits 0 here and REDs this arm: this arm
    # exists because the dead-server argv arm above proved blind to
    # exactly that mutation (the sprint-80 review's hard finding).
    let disc_state = ($repo | path join 'disc-state.json')
    fresh-work-state $disc_state "Make the frobnicator queue bounded"
    let disc_run = (do {
        with-env {FABRO_BIN: (tree-bin), FABRO_SERVER: "http://127.0.0.1:1", FABRO_RUN_ID: $RUN_ID} {
            echo '{"itemKey": "denkhaus/work#1"}' | nu ($SCRIPTS_DIR | path join "reporter.nu") --work-item
        }
    } | complete)
    if $disc_run.exit_code != 1 {
        fail $"a failed fact record must fail the stage — the absorb regression: exit ($disc_run.exit_code)"
    }
    let disc_combined = ($disc_run.stdout + $disc_run.stderr)
    if not ($disc_combined | str contains "observation record failed") {
        fail $"the failure must name the failed fact record: ($disc_combined | str substring 0..200)"
    }

    # (e) Negative arms: no --work-item is a usage error; an empty stdin
    # packet and a malformed packet each hard-fail.
    let neg_state = ($repo | path join 'neg-state.json')
    fresh-work-state $neg_state "Make the frobnicator queue bounded"
    let no_flag = (drive ($SCRIPTS_DIR | path join "reporter.nu") $neg_state)
    if $no_flag.exit_code != 2 { fail "reporter without --work-item must be a usage error (exit 2)" }
    let empty_stdin = (drive ($SCRIPTS_DIR | path join "reporter.nu") $neg_state [--work-item])
    if $empty_stdin.exit_code != 1 { fail "empty stdin packet must hard-fail (exit 1)" }
    let bad_packet = (drive ($SCRIPTS_DIR | path join "reporter.nu") $neg_state [--work-item] '' 'not json')
    if $bad_packet.exit_code != 1 { fail "malformed packet must hard-fail (exit 1)" }
    # The ambient shell may carry FABRO_SERVER (direnv) — the harness
    # arm hides it so the child tests the ABSENCE.
    refuses-missing-server ($SCRIPTS_DIR | path join "reporter.nu") $neg_state [--work-item] '{"itemKey": "denkhaus/work#1"}'

    # The real-parser argv pin (run 01M60NX9W7N65DEPXYN09CE9BR lost its
    # deferred-action filing to this): the work shim is position-blind
    # (nu binds declared flags wherever they appear), so every outbound
    # argv is pinned by driving the REAL fabro binary against a dead
    # server — a clap parse error ('unexpected argument') is
    # distinguishable from the expected transport failure. The reporter
    # emits exactly TWO verb shapes: the observation record (the facts)
    # and the work show (the title resolution).
    do {
        # The fact call needs a binary that HAS the verb: prefer the tree
        # build (the battery host builds it before the quality gate), fall
        # back to whatever `fabro` resolves to. The verb-presence check is
        # part of the pin — an installed CLI that predates the verb names
        # itself (the fork.26/fork.27 skew class).
        let fabro_bin = (tree-bin)
        let dead_server = "http://127.0.0.1:1"
        # the fact call, exactly as the reporter emits it (facts fire
        # first, so this is the one argv a dead-server reporter run
        # reaches)
        let fact_run = (do {
            with-env {FABRO_BIN: $fabro_bin, FABRO_SERVER: $dead_server} {
                echo '{"itemKey": "denkhaus/work#1"}' | nu ($SCRIPTS_DIR | path join "reporter.nu") --work-item
            }
        } | complete)
        let fact_combined = ($fact_run.stdout + $fact_run.stderr)
        if ($fact_combined | str contains 'unexpected argument') {
            fail $"the observation argv drifted out of the real parser's grammar: ($fact_combined | str substring 0..240)"
        }
        if ($fact_combined | str contains 'unrecognized subcommand') {
            fail $"the fact arm's fabro binary predates the observation verb — build the tree CLI \(cargo build -p fabro-cli\) or fix PATH: ($fact_combined | str substring 0..240)"
        }
        if $fact_run.exit_code != 1 {
            fail $"a failed fact record must fail the stage (exit 1): ($fact_combined | str substring 0..240)"
        }
        # the work show shape (the title resolution), driven directly
        # against the dead server: parse errors are clap's, everything
        # else is transport.
        let work_shapes = [
            [work --server $dead_server show --project fabro "denkhaus/work#1" --json]
        ]
        pin-argvs $dead_server $work_shapes
    }
} finally {
    # fail-exits bypass this (nu cannot catch exit); mktemp -t keeps any
    # leak in the system temp dir, never the worktree
    cd $orig_cwd
    rm -rf $repo
}

# The fail gate (see the harness note above): a fail inside the e2e try
# printed and was caught — the marker file is what the battery reads.
end-fail-gate
print "reporter-smoke: ok — the fact machinery plus the facts-only run through the channel shim (visible, not-visible, negatives, argv pins)"


# Sourcing reporter.nu imports its `def main`; nu auto-invokes it after
# the top level runs — exit explicitly so the smoke never reaches it.
exit 0
