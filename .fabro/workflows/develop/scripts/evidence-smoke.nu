#!/usr/bin/env nu
# Smoke test for evidence.nu's PURE helpers (fabro-bfe1), reshaped for
# the METADATA stage (work#1624, ADR-0051): sanitize, diff-sort-key,
# is-loop-path, total, the section printers (stat map, classification,
# anomaly list, worktree, integrity) — exercised over canned fixtures
# without git (helpers that shell out — numstat-rows, run-base,
# worktree-state, claimed-item — are out of scope here; their logic is
# covered by the channel-resolve arms at the bottom and manual
# invocation).
#
# ADR-0051 deletion contract: the capture machinery is DELETED from
# evidence.nu (the git-diff text walks, the inline budgets
# HARD_CAP/EMIT_BUDGET/CHURN_DIFF_LINE_CAP, the blob-ref inlining, the
# inline spec/checks sections, the truncation disclosures). The pins
# below ASSERT the absence: the source must not carry the deleted
# names, and no metadata record may render diff text — the reviewer
# pulls patches via fabro_run_diffs/fabro_blob instead.
#
# The `source` const resolves against THIS file's directory, so the
# script runs from any cwd (closeout-smoke pattern):
#   nu .fabro/workflows/develop/scripts/evidence-smoke.nu

const EVIDENCE = "evidence.nu"
const SCRIPTS_DIR = (path self | path dirname)
# The harness's drive (the residual-boilerplate extraction, #1131):
# evidence keeps its own local fail and negative arm (it predates the
# harness's fail gate), but the drive idiom is shared.
use ($SCRIPTS_DIR | path join '..' '..' '..' 'scripts' 'lib' 'smoke_harness.nu') [drive]
source $EVIDENCE

def fail [what: string]: nothing -> nothing {
    print -e $"evidence-smoke: FAIL — ($what)"
    exit 1
}

# ---------------------------------------------------------------------------
# ADR-0051 absence pins: the deleted capture machinery must not return.
# ---------------------------------------------------------------------------

let ev_src = (open --raw ($SCRIPTS_DIR | path join 'evidence.nu'))
# Code-only scan: nu comments are `#` lines; the header comment NAMES the
# deleted machinery as documentation (ADR-0051's deletion contract wants
# the audit trail), so the pins assert absence from CODE, not prose.
let ev_code = ($ev_src | lines | where {|l| not (($l | str trim) | str starts-with '#') } | str join "\n")
# The inline budgets and their friends (the capture-era size machinery).
for gone in ["HARD_CAP" "EMIT_BUDGET" "CHURN_DIFF_LINE_CAP" "hard cap hit"] {
    if ($ev_code | str contains $gone) {
        fail $"evidence.nu must not carry deleted inline budget machinery: ($gone) — ADR-0051 deletion contract, the reviewer pulls and the stage emits metadata only"
    }
}
# The git-diff text walks and their disclosures.
for gone in ["diff-walk" "capped-diff-walk" "cap-changed-lines" "walk-failure-note" "zero-hunk-note" "truncated:" "UNSEEN"] {
    if ($ev_code | str contains $gone) {
        fail $"evidence.nu must not carry deleted diff-capture machinery: ($gone) — ADR-0051 deletion contract, no diff text rides the metadata"
    }
}
# The blob-ref inlining and the inline spec/checks sections: blob refs
# pass through to the reviewer's pull doors untouched, and the proof of
# record is the check transcript, not an inlined capture section.
for gone in ["resolve-blobrefs" "blob://sha256" "spec-section" "checks-section" "checks-transcript-path"] {
    if ($ev_code | str contains $gone) {
        fail $"evidence.nu must not carry deleted inline-section machinery: ($gone) — ADR-0051 deletion contract, the reviewer pulls diffs and reads the item itself"
    }
}
# Sourced-scope assertion: the deleted commands are really gone (not
# merely renamed in a comment).
if (which resolve-blobrefs | is-not-empty) {
    fail "resolve-blobrefs must not exist in the sourced evidence.nu scope (ADR-0051)"
}
if (which diff-walk | is-not-empty) {
    fail "diff-walk must not exist in the sourced evidence.nu scope (ADR-0051)"
}
if (which cap-changed-lines | is-not-empty) {
    fail "cap-changed-lines must not exist in the sourced evidence.nu scope (ADR-0051)"
}

# ---------------------------------------------------------------------------
# kept pure helpers — the metadata shape
# ---------------------------------------------------------------------------

# sanitize: bare "/word" tokens get backticked (agent skill-reference
# crash guard); uppercase-led tokens and token-free text pass through.
if (sanitize "run in /tmp dir") != "run in `/tmp` dir" {
    fail $"sanitize single token: (sanitize 'run in /tmp dir')"
}
# Two passes catch consecutive tokens — the trailing space of match one
# is the leading space of match two.
if (sanitize "a /b /c d") != "a `/b` `/c` d" {
    fail $"sanitize consecutive tokens: (sanitize 'a /b /c d')"
}
if (sanitize "path /Workspace stays") != "path /Workspace stays" {
    fail $"sanitize uppercase token must pass: (sanitize 'path /Workspace stays')"
}
if (sanitize "no tokens here") != "no tokens here" {
    fail "sanitize token-free text must be unchanged"
}

# diff-sort-key: source files sort before docs (a:/z: prefixes) — the
# reviewer pulls the source patches first.
if (diff-sort-key "lib/main.rs") != "a:lib/main.rs" {
    let source_key = (diff-sort-key 'lib/main.rs')
    fail $"diff-sort-key source prefix: ($source_key)"
}
if (diff-sort-key "docs/x.md") != "z:docs/x.md" {
    let doc_key = (diff-sort-key 'docs/x.md')
    fail $"diff-sort-key doc prefix: ($doc_key)"
}
if ((diff-sort-key "lib/main.rs") > (diff-sort-key "docs/x.md")) {
    fail "diff-sort-key must order source before docs"
}

# is-loop-path: dev-loop machinery paths are loop paths; product code
# and untracked roots are not.
let loop_pos = [".fabro/x" ".seeds/issues.jsonl" "scripts/q.nu" ".mulch/y" "justfile" "AGENTS.md" "CLAUDE.md" ".gitignore"]
for p in $loop_pos {
    if not (is-loop-path $p) { fail $"is-loop-path positive: ($p)" }
}
let loop_neg = ["lib/main.rs" "apps/fabro-web/src/x.ts" "README.md" "justfile.md"]
for p in $loop_neg {
    if (is-loop-path $p) { fail $"is-loop-path negative: ($p)" }
}

# total over a canned numstat fixture (the shape numstat-rows returns):
# binary "-" counts as 0, empty input degrades to 0.
let fixture = [
    {add: "10" del: "2" path: "lib/a.rs"}
    {add: "-"  del: "5" path: "logo.png"}
    {add: "3"  del: "1" path: "docs/b.md"}
]
if (total $fixture "add") != 13 { fail $"total add, binary as 0: (total $fixture 'add')" }
if (total $fixture "del") != 8  { fail $"total del: (total $fixture 'del')" }
if (total [] "add") != 0 { fail "total empty must be 0" }

# ---------------------------------------------------------------------------
# the metadata section printers (work#1624, ADR-0051 shape)
# ---------------------------------------------------------------------------

# stat-map-section: the completeness checklist and pull list — seed
# rows first tagged `seed`, churn rows tagged `churn`, one
# +adds/-dels line per file; empty input renders the none note; the
# header carries the PULL contract (every listed file must be pulled
# and adjudicated).
let map_seed = [{add: "10" del: "2" path: "lib/a.rs"}]
let map_churn = [{add: "1" del: "1" path: "justfile"}]
let map = (stat-map-section $map_seed $map_churn)
if not ($map | str contains "stat map: every changed file since run base") {
    fail "stat-map-section must carry the contract header"
}
if not ($map | str contains "must be pulled and adjudicated") {
    fail "stat-map-section must carry the pull contract (work#1624, ADR-0051)"
}
if not ($map | str contains "seed  +10/-2 lib/a.rs") {
    fail $"stat-map-section seed line: ($map)"
}
if not ($map | str contains "churn +1/-1 justfile") {
    fail $"stat-map-section churn line: ($map)"
}
if not ($map | str starts-with "\n== stat map") {
    fail "stat-map-section must open with its own section head"
}
if not ((stat-map-section [] []) | str contains "(no changed files since run base)") {
    fail "stat-map-section empty input must render the none note"
}

# integrity-section: the header names both bases mechanically and the
# counts line carries the seed/churn totals plus the worktree label.
let integ = (integrity-section "abc1234" "denkhaus/work#1: T" "abc1234 (run base)" $map_seed $map_churn "clean")
if not ($integ | str contains "evidence: base=abc1234 seed=denkhaus/work#1: T diff-base=abc1234 (run base)") {
    fail $"integrity-section must carry the header line: ($integ)"
}
if not ($integ | str contains "integrity: seed-work=1 files +10/-2 | loop-churn=1 files +1/-1 | worktree=clean") {
    fail $"integrity-section must carry the counts line: ($integ)"
}

# seed-work-files-section: the classification's review-scope list —
# source files before docs, one numstat line each, NO diff text.
let files_sec = (seed-work-files-section [
    {add: "1" del: "1" path: "docs/b.md"}
    {add: "10" del: "2" path: "lib/a.rs"}
])
if not ($files_sec | str contains "seed work: changed files (review scope — pull each file's patch)") {
    fail $"seed-work-files-section must carry the pull-scope header: ($files_sec)"
}
let files_lines = ($files_sec | lines | where {|l| $l | str contains "+"})
if (($files_lines | get 0) | str contains "lib/a.rs") == false {
    fail $"seed-work-files-section must list source before docs: ($files_sec)"
}
if ($files_sec | str contains "diff --git") {
    fail "seed-work-files-section must render NO diff text (ADR-0051 — the reviewer pulls)"
}
if not ((seed-work-files-section []) | str contains "(none — no project source changed since run base)") {
    fail "seed-work-files-section empty input must render the none note"
}

# loop-churn-section: counts only, never seed work.
let churn_sec = (loop-churn-section [{add: "1" del: "1" path: "justfile"}])
if not ($churn_sec | str contains "loop churn (dev-loop machinery") {
    fail $"loop-churn-section must carry its header: ($churn_sec)"
}
if not ($churn_sec | str contains "justfile +1/-1") {
    fail $"loop-churn-section must list the numstat line: ($churn_sec)"
}
if not ((loop-churn-section []) | str contains "(none)") {
    fail "loop-churn-section empty input must render the none note"
}

# anomaly-section: NUMSTAT ONLY (work#1624, ADR-0051) — the reviewer
# adjudicates every listed file by pulling its patch, never from
# capture text; empty churn renders the no-anomaly note.
let anom = (anomaly-section [{add: "3" del: "4" path: "scripts/x.nu"}])
if not ($anom | str contains "anomaly: changed files NOT named by the seed spec") {
    fail $"anomaly-section must carry its header: ($anom)"
}
if not ($anom | str contains "adjudicate EVERY listed file") {
    fail $"anomaly-section must carry the adjudication contract: ($anom)"
}
if not ($anom | str contains "scripts/x.nu +3/-4") {
    fail $"anomaly-section must list the numstat line: ($anom)"
}
if ($anom | str contains "diff --git") {
    fail "anomaly-section must render NO diff text (ADR-0051 — the reviewer pulls)"
}
if not ((anomaly-section []) | str contains "(no anomaly files against the run base — loop files changed before the base only)") {
    fail "anomaly-section empty input must render the no-anomaly note"
}

# worktree-section: untracked-file visibility, sanitized.
let wt_sec = (worktree-section ["?? /tmp dir/thing"])
if not ($wt_sec | str contains "working tree") {
    fail $"worktree-section must carry its header: ($wt_sec)"
}
if not ($wt_sec | str contains "NO checkpoint diff") {
    fail $"worktree-section must teach the untracked contract: ($wt_sec)"
}
if not ($wt_sec | str contains "?? `/tmp` dir/thing") {
    fail $"worktree-section must sanitize bare /word tokens: ($wt_sec)"
}
if not ((worktree-section []) | str contains "(clean)") {
    fail "worktree-section empty input must render clean"
}

# spec-named-paths / spec-token-names / spec-names-path (fabro-d76c):
# null/empty seed -> [] (fail-open, pre-d76c split); path-shaped tokens
# are extracted from the description (sentence punctuation stripped);
# glob tokens name their subtree; bare root files match as words.
if (spec-named-paths null) != [] { fail "spec-named-paths null seed must be []" }
if (spec-named-paths {description: ""}) != [] { fail "spec-named-paths empty description must be []" }
let d76c_seed = {description: "Part (a): edit .fabro/workflows/develop/prompts/implementer.md as the fix; also touch .fabro/workflows/** graphs and the justfile, see docs/internal/x.md. Not paths: fabro-70b5, run 01M2K8TECWAHRR79PQ63V2C0ZP, PR #109."}
let d76c_paths = (spec-named-paths $d76c_seed)
if ".fabro/workflows/develop/prompts/implementer.md" not-in $d76c_paths {
    fail $"spec-named-paths must extract the named prompt file: ($d76c_paths)"
}
if "justfile" not-in $d76c_paths { fail $"spec-named-paths must catch bare root justfile: ($d76c_paths)" }
if "docs/internal/x.md" not-in $d76c_paths { fail $"spec-named-paths must strip trailing sentence dot: ($d76c_paths)" }
# Glob token promotes its subtree, exact token matches exactly, a bare
# directory token never promotes children, unrelated paths stay out.
if not (spec-names-path $d76c_paths ".fabro/workflows/develop/prompts/implementer.md") {
    fail "spec-names-path exact token must match"
}
if not (spec-names-path $d76c_paths ".fabro/workflows/loop/scripts/loop-gate.nu") {
    fail "spec-names-path glob token (.fabro/workflows/**) must promote subtree"
}
if (spec-names-path [] "lib/main.rs") { fail "spec-names-path empty set must never match" }
if (spec-names-path [".fabro/workflows"] ".fabro/workflows/loop/scripts/x.nu") {
    fail "spec-names-path bare directory token must not promote children"
}
if (spec-names-path ["docs/a.md"] "docs/b.md") { fail "spec-names-path sibling must not match" }

# classify-rows (fabro-d76c, the original incident reproduced): a product
# run whose seed spec names a loop-path prompt file must classify that
# file as seed work, not anomaly churn; unnamed loop paths stay churn in
# product and flip to seed work in loop; product code stays seed work in
# product and churn (anomaly) in loop unless the spec names it.
let incident_rows = [
    {add: "3" del: "1" path: "lib/apps/fabro-cli/src/main.rs"}
    {add: "5" del: "2" path: ".fabro/workflows/develop/prompts/implementer.md"}
    {add: "1" del: "1" path: ".fabro/workflows/develop/workflow.fabro"}
]
# Incident seed body: names the prompt file ONLY (no glob) — workflow.fabro
# stays unnamed and must remain anomaly churn in the product lane.
let incident_seed = {description: "Part (a): edit .fabro/workflows/develop/prompts/implementer.md as the fix. Basis: run evidence review."}
let incident_spec = (spec-named-paths $incident_seed)
let prod = (classify-rows $incident_rows "product" $incident_spec)
let prod_seed = ($prod.seed | get path)
let prod_churn = ($prod.churn | get path)
if "lib/apps/fabro-cli/src/main.rs" not-in $prod_seed { fail $"product seed work must carry product code: ($prod_seed)" }
if ".fabro/workflows/develop/prompts/implementer.md" not-in $prod_seed {
    fail $"fabro-d76c: spec-named loop path must be seed work in product lane: ($prod_seed)"
}
if ".fabro/workflows/develop/prompts/implementer.md" in $prod_churn {
    fail $"fabro-d76c: spec-named loop path must NOT be anomaly churn: ($prod_churn)"
}
if ".fabro/workflows/develop/workflow.fabro" not-in $prod_churn {
    fail $"unnamed loop path must stay churn in product lane: ($prod_churn)"
}
let lp = (classify-rows $incident_rows "loop" [])
if ".fabro/workflows/develop/workflow.fabro" not-in ($lp.seed | get path) {
    fail $"loop lane: loop assets must be seed work even unnamed: ($lp.seed)"
}
if "lib/apps/fabro-cli/src/main.rs" not-in ($lp.churn | get path) {
    fail $"loop lane: unnamed product code must be anomaly churn: ($lp.churn)"
}
let lp_named = (classify-rows $incident_rows "loop" ["lib/apps/fabro-cli/src/main.rs"])
if "lib/apps/fabro-cli/src/main.rs" not-in ($lp_named.seed | get path) {
    fail $"loop lane: spec-named product path must be seed work: ($lp_named.seed)"
}

# ---------------------------------------------------------------------------
# check-transcript.nu wrapper pins (the recording half — unchanged by
# work#1624: the transcript stays the run's recorded proof)
# ---------------------------------------------------------------------------

# Negative-path pin (the wrapper records the guarded command's OWN exit
# code): run check-transcript.nu as a subprocess with an isolated
# transcript dir (the FABRO_CHECK_TRANSCRIPT_DIR override exists for
# exactly this battery) against a command that exits 7 with stderr —
# the wrapper must exit 7 AND the transcript record must carry
# `-> exit 7` plus the command's stderr, never a later command's code.
# The nested shell matters here: an embedded `exit` must not kill the
# wrapper's own bash before the marker write. The POSIX form this arm
# once used (`sh -c 'echo boom >&2; exit 7'`) is the #1190 trap shape,
# refused at registration since the guard landed — `nu -c` carries the
# same inner-exit semantics through the wrapper's documented escape
# hatch (nu probes) without the double-parsed POSIX payload.
let tdir_pin = (mktemp -d)
let wrapped = (do {
    with-env { FABRO_CHECK_TRANSCRIPT_DIR: $tdir_pin } {
        ^nu $"($SCRIPTS_DIR)/check-transcript.nu" -- "nu -c \"print -e boom; exit 7\""
    }
} | complete)
if $wrapped.exit_code != 7 {
    fail $"negative-path pin: wrapper must pass through exit 7, got ($wrapped.exit_code)"
}
if not ($wrapped.stdout | str contains "-> exit 7") {
    fail $"negative-path pin: streamed record must show exit 7: ($wrapped.stdout)"
}
let pin_files = (ls $"($tdir_pin)/*.md")
let pin_rec = (open --raw ($pin_files | get name | first))
rm -rf $tdir_pin
if not ($pin_rec | str contains "-> exit 7") {
    fail $"negative-path pin: transcript must record exit 7: ($pin_rec)"
}
if not ($pin_rec | str contains "[stderr] boom") {
    fail $"negative-path pin: transcript must carry the command's stderr: ($pin_rec)"
}
if ($pin_rec | str contains "exit-fallback") {
    fail $"negative-path pin: marker capture must not degrade: ($pin_rec)"
}

# Fail-open fallback pin: a guarded command that KILLS the inner bash
# (SIGKILL never runs the marker write) — the wrapper must fail open,
# taking the code from `complete`. Nu encodes the signal as -9 in the
# record (`-> exit -9`); the wrapper's own `exit -9` wraps to a u8
# process code of 247 (256-9) as the parent observes it.
let tdir_fb = (mktemp -d)
let killed = (do {
    with-env { FABRO_CHECK_TRANSCRIPT_DIR: $tdir_fb } {
        ^nu $"($SCRIPTS_DIR)/check-transcript.nu" -- "kill -9 \$BASHPID"
    }
} | complete)
if $killed.exit_code != 247 {
    fail $"fallback pin: wrapper must fail open with the wrapped 247, got ($killed.exit_code)"
}
let fb_files = (ls $"($tdir_fb)/*.md")
let fb_rec = (open --raw ($fb_files | get name | first))
rm -rf $tdir_fb
if not ($fb_rec | str contains "[exit-fallback] marker unreadable — code from complete") {
    fail $"fallback pin: record must carry the fallback line: ($fb_rec)"
}
if not ($fb_rec | str contains "-> exit -9") {
    fail $"fallback pin: record must carry complete's signal code -9: ($fb_rec)"
}
if ($fb_rec | str contains "-> exit 0") {
    fail $"fallback pin: killed bash must never record a green code: ($fb_rec)"
}

# Quoting-death hint pin (nested-shell mangling): a guarded command with
# UNBALANCED quotes (what a nested `sh -c "..."` probe mangled by an
# outer shell layer looks like) makes bash die at parse with exit 2 and
# no marker — the wrapper must fail open, record exit 2, and append the
# `[quoting]` hint pointing at the canonical mktemp nu-probe escape.
let tdir_q = (mktemp -d)
let mangled = (do {
    with-env { FABRO_CHECK_TRANSCRIPT_DIR: $tdir_q } {
        ^nu $"($SCRIPTS_DIR)/check-transcript.nu" -- 'sh -c "echo never-runs; exit 4'
    }
} | complete)
if $mangled.exit_code != 2 {
    fail $"quoting pin: wrapper must pass through bash parse exit 2, got ($mangled.exit_code)"
}
let q_files = (ls $"($tdir_q)/*.md")
let q_rec = (open --raw ($q_files | get name | first))
rm -rf $tdir_q
if not ($q_rec | str contains "-> exit 2") {
    fail $"quoting pin: transcript must record exit 2: ($q_rec)"
}
if not ($q_rec | str contains "[quoting]") {
    fail $"quoting pin: record must carry the quoting hint: ($q_rec)"
}
if not ($q_rec | str contains "mktemp") {
    fail $"quoting pin: hint must name the mktemp nu-probe escape: ($q_rec)"
}

# Nested-shell metachar refusal pin (denkhaus/work#1190): run
# 01M6X3WCQ1's recorded diagnostic — a nested `sh -c` payload whose
# unquoted graphviz arrow the inner shell parsed as a redirect, leaking
# echo's output into the repo-root file `b` that rode the evidence
# commit as seed work. The wrapper refuses the shape at registration:
# exit 64 (EX_USAGE — the command never ran), the REFUSED message names
# the parse-once escapes, and NOTHING lands in the transcript.
let tdir_ref = (mktemp -d)
let refused = (do {
    with-env { FABRO_CHECK_TRANSCRIPT_DIR: $tdir_ref } {
        ^nu $"($SCRIPTS_DIR)/check-transcript.nu" -- 'sh -c "echo digraph G { a -> b } | ... __render-graph"'
    }
} | complete)
if $refused.exit_code != 64 {
    rm -rf $tdir_ref
    fail $"refusal pin: wrapper must refuse the nested-shell metachar trap with 64, got ($refused.exit_code)"
}
if not ($refused.stderr | str contains "REFUSED") {
    rm -rf $tdir_ref
    fail $"refusal pin: refusal must print the REFUSED escape guidance: ($refused.stderr)"
}
if not ($refused.stderr | str contains "Nothing was recorded") {
    rm -rf $tdir_ref
    fail $"refusal pin: refusal must say nothing was recorded: ($refused.stderr)"
}
if ((ls $tdir_ref | length) != 0) {
    rm -rf $tdir_ref
    fail "refusal pin: the refused command must not land in the transcript"
}
rm -rf $tdir_ref

# Re-run stamp pin (a first-attempt failure fixed and re-run must not
# leave two indistinguishable records): the SAME command recorded twice
# into one isolated transcript dir must stamp the second record
# `[re-run 2]` naming the latest record as the proof of record, while
# the first record stays un-stamped; distinct commands never stamp.
let tdir_rr = (mktemp -d)
let rr_first = (do {
    with-env { FABRO_CHECK_TRANSCRIPT_DIR: $tdir_rr } {
        ^nu $"($SCRIPTS_DIR)/check-transcript.nu" -- "sh -c 'exit 0'"
    }
} | complete)
let rr_again = (do {
    with-env { FABRO_CHECK_TRANSCRIPT_DIR: $tdir_rr } {
        ^nu $"($SCRIPTS_DIR)/check-transcript.nu" -- "sh -c 'exit 0'"
    }
} | complete)
let rr_other = (do {
    with-env { FABRO_CHECK_TRANSCRIPT_DIR: $tdir_rr } {
        ^nu $"($SCRIPTS_DIR)/check-transcript.nu" -- "sh -c 'exit 3'"
    }
} | complete)
rm -rf $tdir_rr
if $rr_first.exit_code != 0 or $rr_again.exit_code != 0 {
    fail $"re-run pin: wrapper must pass through exit 0, got first=($rr_first.exit_code) again=($rr_again.exit_code)"
}
if ($rr_first.stdout | str contains "[re-run") {
    fail $"re-run pin: first record must not carry a re-run stamp: ($rr_first.stdout)"
}
if not ($rr_again.stdout | str contains "[re-run 2]") {
    fail $"re-run pin: second record of the same command must be stamped re-run 2: ($rr_again.stdout)"
}
if not ($rr_again.stdout | str contains "proof of record") {
    fail $"re-run pin: stamp must name the latest record as the proof of record: ($rr_again.stdout)"
}
if ($rr_other.stdout | str contains "[re-run") {
    fail $"re-run pin: a distinct command must never be stamped: ($rr_other.stdout)"
}

# ---------------------------------------------------------------------------
# Direct work-item mode through the channel shim (denkhaus/work#990):
# the metadata stage's claim-integrity check is the pin — the CHANNEL
# resolves the item (show) and THIS run's live claim must name it
# (claims). A temp git repo shaped like a run plus a shim state file
# make every arm deterministic; nothing here may ever touch a real
# server. Positive AND negative arms, per the edited-script dry-run rule.
# ---------------------------------------------------------------------------
const RUN_ID = "01SMOKE0000000000000000000"
const SHIM = ($SCRIPTS_DIR | path join '..' '..' '..' 'scripts' 'fabro-work-shim.nu')
let orig_cwd = (pwd)
let erepo = (mktemp -d -t evidence-e2e.XXXXXX)
try {
    cd $erepo
    git init -q -b $"fabro/run/($RUN_ID)" .
    git config user.email smoke@localhost
    git config user.name closeout-smoke
    echo base | save -f base.txt
    git add base.txt
    git commit -qm "initial"

    # (a) Positive: the run holds a live claim on the item -> the
    # metadata header names the CHANNEL-resolved key and title, and the
    # output carries NO diff text (ADR-0051: metadata only).
    let ok_state = ($erepo | path join 'ok-state.json')
    {items: [{itemKey: "denkhaus/work#1", title: "Channel resolved title", body: "Edit lib/a.rs", state: "open", labels: []}],
     claims: [{claimId: "c1", itemKey: "denkhaus/work#1", runId: $RUN_ID, lane: "", state: "granted"}],
     nextKey: 8001, log: []} | to json --raw | save -f $ok_state
    let ev_ok = (drive ($SCRIPTS_DIR | path join "evidence.nu") $ok_state [--work-item] '' '{"itemKey":"denkhaus/work#1"}')
    if $ev_ok.exit_code != 0 {
        fail $"evidence --work-item positive run failed: ($ev_ok.stderr | str substring 0..240)"
    }
    if not ($ev_ok.stdout | str contains "seed=denkhaus/work#1: Channel resolved title") {
        fail "evidence header must name the channel-resolved item key and title"
    }
    if not ($ev_ok.stdout | str contains "stat map: every changed file since run base") {
        fail "evidence output must carry the stat map record"
    }
    if ($ev_ok.stdout | str contains "diff --git") {
        fail "evidence output must carry NO diff text (work#1624, ADR-0051 — the reviewer pulls via fabro_run_diffs)"
    }

    # (b) Foreign claim: another run holds the item -> hard-fail.
    let foreign_state = ($erepo | path join 'foreign-state.json')
    {items: [{itemKey: "denkhaus/work#1", title: "T", body: "b", state: "open", labels: []}],
     claims: [{claimId: "c9", itemKey: "denkhaus/work#1", runId: "01FOREIGN00000000000000000", lane: "loop", state: "active"}],
     nextKey: 8001, log: []} | to json --raw | save -f $foreign_state
    let ev_foreign = (drive ($SCRIPTS_DIR | path join "evidence.nu") $foreign_state [--work-item] '' '{"itemKey":"denkhaus/work#1"}')
    if $ev_foreign.exit_code == 0 {
        fail "a run without a live claim on the item must hard-fail (the integrity check)"
    }
    if not ($ev_foreign.stderr | str contains "no live claim") {
        fail $"the foreign-claim refusal must name the missing claim: ($ev_foreign.stderr | str substring 0..160)"
    }

    # (c) Unresolvable item: the channel cannot show it -> hard-fail.
    let ghost_state = ($erepo | path join 'ghost-state.json')
    {items: [], claims: [{claimId: "c2", itemKey: "denkhaus/work#404", runId: $RUN_ID, lane: "", state: "granted"}], nextKey: 8001, log: []} | to json --raw | save -f $ghost_state
    let ev_ghost = (drive ($SCRIPTS_DIR | path join "evidence.nu") $ghost_state [--work-item] '' '{"itemKey":"denkhaus/work#404"}')
    if $ev_ghost.exit_code == 0 {
        fail "an unresolvable item must hard-fail (the channel is the resolver)"
    }

    # (d) Malformed packet -> hard-fail before ANY channel call.
    let bad_state = ($erepo | path join 'bad-state.json')
    {items: [], claims: [], nextKey: 8001, log: []} | to json --raw | save -f $bad_state
    let ev_bad = (drive ($SCRIPTS_DIR | path join "evidence.nu") $bad_state [--work-item] '' 'not-json')
    if $ev_bad.exit_code == 0 {
        fail "evidence --work-item must hard-fail on a malformed packet — a silent fallback would reintroduce the wrong-item capture class"
    }
    if not ($ev_bad.stderr | str contains "granted-item packet") {
        fail $"the malformed-packet refusal must name the packet: ($ev_bad.stderr | str substring 0..160)"
    }
    let bad_log = (open --raw $bad_state | from json).log
    if ($bad_log | length) != 0 {
        fail "a malformed packet must not reach the channel"
    }

    # (e) A missing FABRO_SERVER is refused loudly.
    let ev_noserver = (do {
        with-env {FABRO_BIN: $SHIM, FABRO_WORK_SHIM_STATE: $bad_state} {
            hide-env FABRO_SERVER
            echo '{"itemKey":"denkhaus/work#1"}' | nu ($SCRIPTS_DIR | path join "evidence.nu") --work-item
        }
    } | complete)
    if $ev_noserver.exit_code != 1 {
        fail "a missing FABRO_SERVER must be refused loudly (exit 1)"
    }
} finally {
    # fail-exits bypass this (nu cannot catch exit); mktemp -t keeps any
    # leak in the system temp dir, never the worktree
    cd $orig_cwd
    rm -rf $erepo
}

print "evidence-smoke: ok — absence-pins (budgets/diff-walks/blob-inlining/inline-sections deleted, ADR-0051)/sanitize/diff-sort-key/is-loop-path/total/stat-map/integrity/seed-work-files/loop-churn/anomaly-list/worktree/spec-named-paths/classify-rows/wrapper-pins/channel-resolve-and-claim-integrity verified"

# Sourcing evidence.nu imports its `def main`; nu auto-invokes it after
# the top level runs — exit explicitly so the smoke never reaches it.
exit 0
