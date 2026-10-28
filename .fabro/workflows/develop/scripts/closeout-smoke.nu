#!/usr/bin/env nu
# Smoke test for closeout.nu's Dockerfile-touch warning (fabro-6f6e).
# Exercises the pure glob/filter logic — dockerfile-hits over a path
# list — without git; the full-script path is a manual invocation check
# (documented on the seed). The `source` const resolves against THIS
# file's directory, so the script runs from any cwd:
#   nu .fabro/workflows/develop/scripts/closeout-smoke.nu

const CLOSEOUT = "closeout.nu"
const SCRIPTS_DIR = (path self | path dirname)
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
let toks = (demand-tokens "Closeout closure discipline: seeds close only when the seed demand is visible in the run diff, else a documented reason is mandatory")
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

# --- Reviewer-journal non-blocking sweep (fabro-22fa) ------------------
# Marker matching: explicit non-blocking wording (both phrasings,
# case-insensitive); a blocking statement stays out.
if not (is-nonblocking "Non-blocking: octal escapes mis-decode, harmless today") {
    fail "is-nonblocking dropped the 'Non-blocking:' marker"
}
if not (is-nonblocking "Noted but not blocking: retry count is per-process") {
    fail "is-nonblocking dropped the 'noted but not blocking' phrasing"
}
if (is-nonblocking "this defect is blocking, do not approve") {
    fail "is-nonblocking matched a blocking statement"
}

# Journal parsing: reviewer-node observations only; other nodes and
# non-matching observations never file.
let jl = (
    nonblocking-from-journal (
        [
            '{"node":"implementer","data":{"observations":["non-blocking mention in the wrong stage"]}}'
            '{"node":"reviewer","data":{"observations":["Verified the diff hunk-by-hunk."]}}'
            '{"node":"reviewer","data":{"observations":["Non-blocking: Latin-1 mis-decode of quoted paths, harmless for the sole consumer."]}}'
            '{"node":"tester","data":{}}'
            'not json at all'
        ] | str join "\n"
    )
)
if ($jl | length) != 1 {
    fail $"nonblocking-from-journal wrong count: ($jl | to json -r)"
}
if not ($jl.0 | str contains "Latin-1") {
    fail "nonblocking-from-journal dropped the finding text"
}

# Null path, fixture journal (tmp file): a real reviewer record with NO
# non-blocking findings must yield an empty list — and an empty list
# means zero seeds create calls in the sweep loop. Also proves the
# missing-journal path degrades to empty.
let tmp = (mktemp -t closeout-null.XXXXXX.jsonl)
'{"node":"reviewer","data":{"painpoints":[],"observations":["Clean approve: diff verified against spec, nothing residual."]}}' | save -f $tmp
if ((journal-nonblocking $tmp) | is-not-empty) {
    fail "null-path fixture journal produced findings (would file seeds)"
}
rm -f $tmp
if ((journal-nonblocking "/nonexistent/.fabro/journal/none.jsonl") | is-not-empty) {
    fail "missing journal did not degrade to empty"
}

# Filed-seed title: excerpt + provenance, bounded length.
let xs = (1..200 | each {"x"} | str join)
let ft = (finding-title $xs "fabro-22fa")
if not ($ft | str starts-with "Reviewer residual (non-blocking) from fabro-22fa:") {
    fail $"finding-title missing prefix: ($ft)"
}
if ($ft | str length) > 140 {
    fail "finding-title unbounded excerpt"
}

# Regression guard (fabro-22fa): finding-title and the null path stay
# unchanged next to the channel filing (labels ride the create call,
# pinned by the end-to-end arms below).
if (finding-title "Short finding" "fabro-22fa") != "Reviewer residual \(non-blocking\) from fabro-22fa: Short finding" {
    fail "finding-title shape drifted"
}
let long = (0..79 | each { "x" } | str join)
if ((finding-title $long "fabro-22fa" | str length) != 70 + ("Reviewer residual \(non-blocking\) from fabro-22fa: " | str length)) {
    fail "finding-title truncation drifted"
}
if ((journal-nonblocking "/nonexistent/.fabro/journal/none2.jsonl") | is-not-empty) {
    fail "null-path journal degradation drifted"
}

# --- Deferred-action sweep (fabro-7aac) --------------------------------
# Marker matching: `deferred-action:` at the START, case-insensitive,
# whitespace-tolerant; mid-sentence mentions and plain observations
# never match.
if not (is-deferred-action "deferred-action: regen the TS client locally (openapi-generator absent in sandbox)") {
    fail "is-deferred-action dropped the start marker"
}
if not (is-deferred-action "  Deferred-Action: confirm bun build with user") {
    fail "is-deferred-action is case/whitespace sensitive"
}
if (is-deferred-action "we noted a deferred-action: mid-sentence mention") {
    fail "is-deferred-action matched a mid-sentence marker mention"
}
if (is-deferred-action "ordinary observation, no marker") {
    fail "is-deferred-action matched a plain observation"
}

# Marker strip: deferred-text yields the action text after the marker;
# non-matching input passes through (defensive path).
if (deferred-text "deferred-action: run bun generate locally") != "run bun generate locally" {
    fail $"deferred-text strip wrong: (deferred-text 'deferred-action: run bun generate locally')"
}
if (deferred-text "plain text") != "plain text" {
    fail "deferred-text passthrough broken"
}

# Journal parsing: implementer-node observations only; reviewer-node
# marker observations and non-matching implementer observations never
# file; unparsable lines degrade away.
let dj = (
    deferred-from-journal (
        [
            '{"node":"reviewer","data":{"observations":["deferred-action: reviewer is not the sweep source"]}}'
            '{"node":"implementer","data":{"observations":["wrote the sweep, nothing deferred"]}}'
            '{"node":"implementer","data":{"observations":["deferred-action: regen fabro-api-client via bun run generate (sandbox lacks java/openapi-generator)"]}}'
            '{"node":"tester","data":{}}'
            'not json at all'
        ] | str join "\n"
    )
)
if ($dj | length) != 1 {
    fail $"deferred-from-journal wrong count: ($dj | to json -r)"
}
if not ($dj.0 | str contains "bun run generate") {
    fail "deferred-from-journal dropped the action text"
}

# Null path, fixture journal (tmp file): a real implementer record with
# NO marker observations must yield an empty list — and an empty list
# means zero seeds create calls in the sweep loop. Also proves the
# missing-journal path degrades to empty.
let dtmp = (mktemp -t closeout-deferred-null.XXXXXX.jsonl)
'{"node":"implementer","data":{"painpoints":[],"observations":["Clean pass: sweep added, smoke green, nothing deferred."]}}' | save -f $dtmp
if ((journal-deferred $dtmp) | is-not-empty) {
    fail "deferred null-path fixture journal produced actions (would file seeds)"
}
rm -f $dtmp
if ((journal-deferred "/nonexistent/.fabro/journal/none3.jsonl") | is-not-empty) {
    fail "deferred missing journal did not degrade to empty"
}

# Filed-seed title: excerpt + provenance, bounded length.
let dft = (deferred-title "regen the generated TS client locally with the pinned generator" "fabro-7aac")
if not ($dft | str starts-with "Deferred follow-up from fabro-7aac:") {
    fail $"deferred-title missing prefix: ($dft)"
}
if ($dft | str length) > 140 {
    fail "deferred-title unbounded excerpt"
}

# No-op payloads (fabro-7ece): a marker observation whose stripped text is
# empty or exactly `none` means "nothing deferred" — the sweep must skip it
# (log, file nothing), never file a junk residual seed.
if not (deferred-action-empty "none") { fail "deferred-action-empty dropped literal none" }
if not (deferred-action-empty "  NONE  ") { fail "deferred-action-empty is case/whitespace sensitive" }
if not (deferred-action-empty "") { fail "deferred-action-empty dropped empty payload" }
if not (deferred-action-empty "   ") { fail "deferred-action-empty dropped whitespace-only payload" }
if (deferred-action-empty "regen the TS client locally") { fail "deferred-action-empty swallowed a real action" }

# Filable set: what the sweep loop actually iterates after the none/empty
# skip — a journal with ONLY none/empty marker observations yields zero
# filings; adding one real marker observation yields exactly that action.
let noneonly = (
    deferred-from-journal (
        [
            '{"node":"implementer","data":{"observations":["deferred-action: none"]}}'
            '{"node":"implementer","data":{"observations":["Deferred-Action:   "]}}'
        ] | str join "\n"
    )
    | where {|o| not (deferred-action-empty (deferred-text $o))}
)
if ($noneonly | is-not-empty) {
    fail $"none-marker journal still yields filings: ($noneonly | to json -r)"
}
let mixed = (
    deferred-from-journal (
        [
            '{"node":"implementer","data":{"observations":["deferred-action: none"]}}'
            '{"node":"implementer","data":{"observations":["deferred-action: regen the TS client locally"]}}'
        ] | str join "\n"
    )
    | where {|o| not (deferred-action-empty (deferred-text $o))}
)
if ($mixed | length) != 1 or ((deferred-text $mixed.0) != "regen the TS client locally") {
    fail $"mixed journal filable set wrong: ($mixed | to json -r)"
}

# --- EXEMPTION-arm sweep (fabro-534e) ----------------------------------
# Marker matching: `exemption:` at the START, case-insensitive,
# whitespace-tolerant; mid-sentence mentions and plain observations
# never match.
if not (is-exemption "exemption: run just image-release and apply the tofu stack (infra is outside the run)") {
    fail "is-exemption dropped the start marker"
}
if not (is-exemption "  Exemption: confirm the deploy with the on-call human") {
    fail "is-exemption is case/whitespace sensitive"
}
if (is-exemption "the brief carried an exemption: mid-sentence mention") {
    fail "is-exemption matched a mid-sentence marker mention"
}
if (is-exemption "ordinary planner observation, no marker") {
    fail "is-exemption matched a plain observation"
}

# Marker strip: exemption-text yields the arm text after the marker;
# non-matching input passes through (defensive path).
if (exemption-text "exemption: run just image-release") != "run just image-release" {
    fail $"exemption-text strip wrong: (exemption-text 'exemption: run just image-release')"
}
if (exemption-text "plain text") != "plain text" {
    fail "exemption-text passthrough broken"
}

# Journal parsing: planner-node observations only; implementer-node
# marker observations and non-matching planner observations never file;
# unparsable lines degrade away.
let ej = (
    exemptions-from-journal (
        [
            '{"node":"implementer","data":{"observations":["exemption: implementer is not the sweep source"]}}'
            '{"node":"planner","data":{"observations":["claimed the seed, nothing exempted"]}}'
            '{"node":"planner","data":{"observations":["exemption: deploy the landed fix via image-release plus tofu apply — user/infra action"]}}'
            '{"node":"reviewer","data":{}}'
            'not json at all'
        ] | str join "\n"
    )
)
if ($ej | length) != 1 {
    fail $"exemptions-from-journal wrong count: ($ej | to json -r)"
}
if not ($ej.0 | str contains "tofu apply") {
    fail "exemptions-from-journal dropped the arm text"
}

# Null path, fixture journal (tmp file): a real planner record with NO
# marker observations must yield an empty list — and an empty list
# means zero seeds create calls in the sweep loop. Also proves the
# missing-journal path degrades to empty.
let etmp = (mktemp -t closeout-exemption-null.XXXXXX.jsonl)
'{"node":"planner","data":{"painpoints":[],"observations":["Clean claim: brief written, nothing exempted."]}}' | save -f $etmp
if ((journal-exemptions $etmp) | is-not-empty) {
    fail "exemption null-path fixture journal produced arms (would file seeds)"
}
rm -f $etmp
if ((journal-exemptions "/nonexistent/.fabro/journal/none4.jsonl") | is-not-empty) {
    fail "exemption missing journal did not degrade to empty"
}

# Filed-seed title: excerpt + provenance, bounded length.
let eft = (exemption-title "release the toolchain image and apply the deployment stack so the fix reaches production" "fabro-534e")
if not ($eft | str starts-with "EXEMPTION arm from fabro-534e:") {
    fail $"exemption-title missing prefix: ($eft)"
}
if ($eft | str length) > 140 {
    fail "exemption-title unbounded excerpt"
}

# --- Direct work-item mode through the channel shim (denkhaus/work#990) --
# The full-script path IS the pin: a temp git repo (deterministic diff
# for the PARK gate), a shim state file, a fixture journal — and the
# subprocess run proves the packet validation, the sweeps' DIRECT
# filings (title, body-on-stdin, comma-joined labels), the deterministic
# fork-8 dedupe, the PARK arm, and the claim-shaped close call. Positive
# AND negative arms, per the edited-script dry-run rule. The shim path
# resolves ABSOLUTE from the smoke's own location — the subprocess runs
# in the temp repo, a relative path would not exist there.
const SHIM = ($SCRIPTS_DIR | path join '..' '..' '..' 'scripts' 'fabro-work-shim.nu')

def shim-env [state: string]: nothing -> record {
    {FABRO_BIN: $SHIM, FABRO_SERVER: "http://127.0.0.1:1", FABRO_WORK_SHIM_STATE: $state}
}

def fresh-state [path: string, title: string]: nothing -> nothing {
    {items: [{itemKey: "denkhaus/work#1", title: $title, body: "the item body", state: "open", labels: []}],
     claims: [],
     nextKey: 8001,
     log: []} | to json --raw | save -f $path
}

# Temp git repo shaped like a RUN: branch fabro/run/<id>, an initial
# commit, then the checkpoint commit whose subject carries the run-id
# marker (run-base anchors on it) with the title tokens in its diff.
const RUN_ID = "01SMOKE0000000000000000000"
let orig_cwd = (pwd)
let repo = (mktemp -d -t closeout-e2e.XXXXXX)
try {
    cd $repo
    git init -q -b $"fabro/run/($RUN_ID)" .
git config user.email smoke@localhost
    git config user.name closeout-smoke
    mkdir .fabro/journal
    echo initial | save -f base.txt
    git add base.txt
    git commit -qm "initial"
    "frobnicator queue bounded: the deterministic implementing change" | save -f impl.txt
    git add impl.txt
    git commit -qm $"fabro\(($RUN_ID)\): the fixture implementing checkpoint"

    # (a) PARK arm: title tokens absent from the diff -> no close, item
    # stays open, ZERO tracker mutations in the log.
    let park_state = ($repo | path join 'park-state.json')
    fresh-state $park_state "Rewrite the quantum flux capacitor harness"
    let park_run = (do {
        with-env (shim-env $park_state) {
            echo '{"itemKey": "denkhaus/work#1"}' | nu ($SCRIPTS_DIR | path join "closeout.nu") --work-item
        }
    } | complete)
    if $park_run.exit_code != 0 {
        fail $"PARK arm must exit 0 \(a park is a hold, not a failure\): ($park_run.stderr | str substring 0..200)"
    }
    if not ($park_run.stdout | str contains "PARK") {
        fail $"PARK arm must print the park note: ($park_run.stdout | str substring 0..200)"
    }
    let park_state_after = (open --raw $park_state | from json)
    if (($park_state_after.log | where {|e| $e.argv.1 != "show"} | length) != 0) {
        fail "PARK arm must not mutate the tracker (only the title show is allowed)"
    }
    if (($park_state_after.items | where state == "open") | length) != 1 {
        fail "PARK arm must leave the item open"
    }

    # (b) Happy path: title tokens present, a journal carrying one finding,
    # one deferred action, one exemption -> three DIRECT filings with the
    # right labels, then the close.
    let happy_state = ($repo | path join 'happy-state.json')
    fresh-state $happy_state "Make the frobnicator queue bounded"
    [
        '{"node":"reviewer","data":{"observations":["Non-blocking: the cache key could collide under retries, harmless today."]}}'
        '{"node":"implementer","data":{"observations":["deferred-action: regen the TS client locally (no java in sandbox)"]}}'
        '{"node":"planner","data":{"observations":["exemption: run just image-release plus tofu apply (infra action)"]}}'
    ] | str join "\n" | save -f $"($repo)/.fabro/journal/($RUN_ID).jsonl"
    let happy_run = (do {
        with-env (shim-env $happy_state) {
            echo '{"itemKey": "denkhaus/work#1"}' | nu ($SCRIPTS_DIR | path join "closeout.nu") --work-item
        }
    } | complete)
    if $happy_run.exit_code != 0 {
        fail $"happy path failed: ($happy_run.stderr | str substring 0..300)"
    }
    let happy_after = (open --raw $happy_state | from json)
    let filed = ($happy_after.items | where {|it| $it.itemKey != "denkhaus/work#1"})
    if ($filed | length) != 3 {
        fail $"happy path must file exactly three follow-ups, got: ($filed | get itemKey | to json -r)"
    }
    let residual_filed = ($filed | where {|it| ("residual" in $it.labels) and not ("ops" in $it.labels)})
    if ($residual_filed | length) != 2 {
        fail $"finding + deferred filings must carry the residual label: ($residual_filed | to json -r)"
    }
    let ops_filed = ($filed | where {|it| "ops" in $it.labels})
    if ($ops_filed | length) != 1 {
        fail "the exemption filing must carry the ops label"
    }
    if not ($ops_filed.0.body | str contains "image-release") {
        fail "the exemption filing body must carry the arm text"
    }
    if not ($happy_run.stdout | str contains "closed denkhaus/work#1") {
        fail $"happy path must close the claimed item: ($happy_run.stdout | str substring 0..300)"
    }
    let close_calls = ($happy_after.log | where {|e| $e.argv.1 == "close"})
    if ($close_calls | length) != 1 {
        fail "exactly one close call"
    }
    if not ($close_calls.0.flags.reason | str contains $"run \(($RUN_ID)\) after reviewer approval") {
        fail $"the close reason must name the run and the approval: ($close_calls.0.flags.reason | str substring 0..160)"
    }
    # The filings' bodies travelled on STDIN (never on a command line).
    for f in $filed {
        if ($f.body | is-empty) { fail "a filed item lost its body (stdin transport broken)" }
    }

    # (c) Dedupe arm: a state already carrying the finding's title -> the
    # sweep skips it, exactly one NEW filing lands.
    let dedupe_state = ($repo | path join 'dedupe-state.json')
    fresh-state $dedupe_state "Make the frobnicator queue bounded"
    {items: [
        {itemKey: "denkhaus/work#1", title: "Make the frobnicator queue bounded", body: "the item body", state: "open", labels: []}
        {itemKey: "denkhaus/work#7999", title: (finding-title "Non-blocking: the cache key could collide under retries, harmless today." "denkhaus/work#1"), body: $"Prior filing from this run. Basis: run \(($RUN_ID)\), closed item denkhaus/work#1", state: "open", labels: ["residual"]}
    ], claims: [], nextKey: 8001, log: []} | to json --raw | save -f $dedupe_state
    let dedupe_run = (do {
        with-env (shim-env $dedupe_state) {
            echo '{"itemKey": "denkhaus/work#1"}' | nu ($SCRIPTS_DIR | path join "closeout.nu") --work-item
        }
    } | complete)
    if $dedupe_run.exit_code != 0 {
        fail $"dedupe arm failed: ($dedupe_run.stderr | str substring 0..300)"
    }
    if not ($dedupe_run.stdout | str contains "skipped already-filed reviewer residual") {
        fail $"dedupe arm must skip the already-filed finding: ($dedupe_run.stdout | str substring 0..300)"
    }
    let dedupe_after = (open --raw $dedupe_state | from json)
    if (($dedupe_after.items | where {|it| $it.itemKey not-in ["denkhaus/work#1" "denkhaus/work#7999"]}) | length) != 2 {
        fail "dedupe arm must file exactly the two NEW follow-ups (deferred + exemption), not the finding again"
    }

    # (d) Negative arms: no --work-item is a usage error; an empty stdin
    # packet and a malformed packet each hard-fail.
    let neg_state = ($repo | path join 'neg-state.json')
    fresh-state $neg_state "Make the frobnicator queue bounded"
    let no_flag = (do {
        with-env (shim-env $neg_state) { echo "" | nu ($SCRIPTS_DIR | path join "closeout.nu") }
    } | complete)
    if $no_flag.exit_code != 2 { fail "closeout without --work-item must be a usage error (exit 2)" }
    let empty_stdin = (do {
        with-env (shim-env $neg_state) { echo "" | nu ($SCRIPTS_DIR | path join "closeout.nu") --work-item }
    } | complete)
    if $empty_stdin.exit_code != 1 { fail "empty stdin packet must hard-fail (exit 1)" }
    let bad_packet = (do {
        with-env (shim-env $neg_state) { echo "not json" | nu ($SCRIPTS_DIR | path join "closeout.nu") --work-item }
    } | complete)
    if $bad_packet.exit_code != 1 { fail "malformed packet must hard-fail (exit 1)" }
    let no_server = (do {
        with-env {FABRO_BIN: $SHIM, FABRO_WORK_SHIM_STATE: $neg_state} {
            # the ambient shell may carry FABRO_SERVER (direnv) — the arm
            # must test the ABSENCE, so drop it for the child
            hide-env FABRO_SERVER
            echo '{"itemKey": "denkhaus/work#1"}' | nu ($SCRIPTS_DIR | path join "closeout.nu") --work-item
        }
    } | complete)
    if $no_server.exit_code != 1 { fail "a missing FABRO_SERVER must be refused loudly (exit 1)" }

    # The real-parser argv pin (run 01M60NX9W7N65DEPXYN09CE9BR lost its
    # deferred-action filing to this): the work shim is position-blind
    # (nu binds declared flags wherever they appear), so the create argv
    # is pinned by driving closeout.nu END TO END with the REAL fabro
    # binary against a dead server — the park gate degrades open, the
    # dedupe degrades off, the sweeps still fire, and a clap parse error
    # ('unexpected argument', the pre-fix --project-before-the-verb
    # shape) is distinguishable from the expected transport failure.
    do {
        let dead_env = {FABRO_BIN: (which fabro | get 0.path), FABRO_SERVER: "http://127.0.0.1:1"}
        let parse_run = (do {
            with-env $dead_env {
                echo '{"itemKey": "denkhaus/work#1"}' | nu ($SCRIPTS_DIR | path join "closeout.nu") --work-item
            }
        } | complete)
        let combined = ($parse_run.stdout + $parse_run.stderr)
        if ($combined | str contains 'unexpected argument') {
            fail $"an outbound work argv drifted out of the real parser's grammar: ($combined | str substring 0..240)"
        }
        print 'ok: every outbound work argv parses against the real CLI grammar'
    }
} finally {
    # fail-exits bypass this (nu cannot catch exit); mktemp -t keeps any
    # leak in the system temp dir, never the worktree
    cd $orig_cwd
    rm -rf $repo
}

print "closeout-smoke: ok — sweep logic plus the direct work-item arms through the channel shim (park, filings, dedupe, close, negatives)"


# Sourcing closeout.nu imports its `def main`; nu auto-invokes it after
# the top level runs — exit explicitly so the smoke never reaches it.
exit 0

