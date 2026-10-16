#!/usr/bin/env nu
# Smoke test for evidence.nu's PURE helpers (fabro-bfe1): sanitize,
# resolve-blobrefs, diff-sort-key, is-loop-path, total, stat-map-section
# — exercised over
# canned fixtures without git (helpers that shell out — numstat-rows,
# run-base, worktree-state, claimed-seed — are out of scope here;
# their logic is covered by the parse check in scripts/qualitygate.nu
# and manual invocation). The `source` const resolves against THIS
# file's directory, so the script runs from any cwd (closeout-smoke
# pattern):
#   nu .fabro/workflows/develop/scripts/evidence-smoke.nu

const EVIDENCE = "evidence.nu"
const SCRIPTS_DIR = (path self | path dirname)
source $EVIDENCE

def fail [what: string]: nothing -> nothing {
    print -e $"evidence-smoke: FAIL — ($what)"
    exit 1
}

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

# resolve-blobrefs: no refs -> unchanged; resolvable ref (blob file
# materialized in cwd's .fabro/blobs/) -> inlined verbatim; unresolvable
# ref -> passes through as the link, never dropped.
if (resolve-blobrefs "plain text no refs") != "plain text no refs" {
    fail "resolve-blobrefs no-refs text must be unchanged"
}
let repo_root = $env.PWD
let tmp = (mktemp -d)
let sha = "1111111111111111111111111111111111111111111111111111111111111111"
mkdir $"($tmp)/.fabro/blobs"
"BLOB-CONTENT-HERE" | save --force $"($tmp)/.fabro/blobs/($sha).json"
cd $tmp
let inlined = (resolve-blobrefs $"pre blob://sha256/($sha) post")
cd $repo_root
rm -rf $tmp
if $inlined != "pre BLOB-CONTENT-HERE post" {
    fail $"resolve-blobrefs resolvable ref must inline: ($inlined)"
}
let missing = "2222222222222222222222222222222222222222222222222222222222222222"
let passthrough = (resolve-blobrefs $"see blob://sha256/($missing) end")
if $passthrough != $"see blob://sha256/($missing) end" {
    fail $"resolve-blobrefs unresolvable ref must pass through: ($passthrough)"
}

# diff-sort-key: source files sort before docs (a:/z: prefixes) — the
# reviewer sees the complete source diff first.
if (diff-sort-key "lib/main.rs") != "a:lib/main.rs" {
    fail $"diff-sort-key source prefix: (diff-sort-key 'lib/main.rs')"
}
if (diff-sort-key "docs/x.md") != "z:docs/x.md" {
    fail $"diff-sort-key doc prefix: (diff-sort-key 'docs/x.md')"
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
if (total $fixture "add") != 13 { fail $"total add (binary as 0): (total $fixture 'add')" }
if (total $fixture "del") != 8  { fail $"total del: (total $fixture 'del')" }
if (total [] "add") != 0 { fail "total empty must be 0" }

# stat-map-section (fabro-b798): top-of-capture completeness map —
# seed rows first tagged `seed`, churn rows tagged `churn`, one
# +adds/-dels line per file; empty input renders the none note.
let map_seed = [{add: "10" del: "2" path: "lib/a.rs"}]
let map_churn = [{add: "1" del: "1" path: "justfile"}]
let map = (stat-map-section $map_seed $map_churn)
if not ($map | str contains "stat map: every changed file since run base") {
    fail "stat-map-section must carry the contract header"
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

# checks-transcript-path: pure builder — run id lands in the file name
# under the shared transcript dir (check-transcript.nu writes it).
if (checks-transcript-path "abc123") != "/tmp/fabro-check-transcript/abc123.md" {
    fail $"checks-transcript-path: (checks-transcript-path 'abc123')"
}

# checks-section: absent file -> "" (no noise for runs without checks);
# present transcript -> section head + sanitized body; bare /word tokens
# in the body get backticked like every other emitted text.
if (checks-section "/tmp/fabro-check-transcript/definitely-missing.md") != "" {
    fail "checks-section missing file must be empty"
}
let tmp2 = (mktemp -d)
let tpath = $"($tmp2)/checks.md"
"$ just lint-nu\nall green\n-> exit 0\n\nrun in /tmp dir\n-> exit 0" | save --force $tpath
let sec = (checks-section $tpath)
rm -rf $tmp2
if not ($sec | str contains "recorded checks: per-criterion transcript") {
    fail "checks-section must carry the contract header"
}
if not ($sec | str contains "just lint-nu") {
    fail "checks-section must inline the transcript body"
}
if not ($sec | str contains "`/tmp` dir") {
    fail $"checks-section must sanitize bare /word tokens: ($sec)"
}

# Negative-path pin (the wrapper records the guarded command's OWN exit
# code): run check-transcript.nu as a subprocess with an isolated
# transcript dir (the FABRO_CHECK_TRANSCRIPT_DIR override exists for
# exactly this battery) against a command that exits 7 with stderr —
# the wrapper must exit 7 AND the transcript record must carry
# `-> exit 7` plus the command's stderr, never a later command's code.
let tdir_pin = (mktemp -d)
let wrapped = (do {
    with-env { FABRO_CHECK_TRANSCRIPT_DIR: $tdir_pin } {
        ^nu $"($SCRIPTS_DIR)/check-transcript.nu" -- "sh -c 'echo boom >&2; exit 7'"
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

# cap-changed-lines (fabro-020b): a diff within the cap passes through
# unchanged (cut 0); past the cap, exactly the first `cap` changed lines
# are kept, the tail is dropped, and the disclosure counts the cut;
# ---/+++ file headers and @@ hunk headers never count as changed lines.
let cap_fixture = "diff --git a/x b/x\nindex 1..2 100644\n--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n ctx\n-old1\n+new1\n@@ -5,2 +5,2 @@\n ctx2\n-old2\n+new2"
let cap_under = (cap-changed-lines $cap_fixture 4)
if $cap_under.cut != 0 { fail $"cap-changed-lines within cap must cut 0: ($cap_under.cut)" }
if $cap_under.body != $cap_fixture { fail "cap-changed-lines within cap must pass through unchanged" }
let cap_over = (cap-changed-lines $cap_fixture 3)
if $cap_over.cut != 1 { fail $"cap-changed-lines past cap must count the cut: ($cap_over.cut)" }
if not ($cap_over.body | str contains "truncated: 1 further changed lines not shown") {
    fail $"cap-changed-lines must disclose the cut count: ($cap_over.body)"
}
if ($cap_over.body | str contains "+new2") {
    fail "cap-changed-lines must drop the changed lines past the cap"
}
if not ($cap_over.body | str contains "+new1") {
    fail "cap-changed-lines must keep the changed lines within the cap"
}
if not ($cap_over.body | str contains "@@ -5,2 +5,2 @@") {
    fail "cap-changed-lines must keep hunk headers inside the kept region"
}
if (cap-changed-lines $cap_fixture 0).cut != 4 {
    fail "cap-changed-lines cap 0 must count every changed line as cut"
}

print "evidence-smoke: ok — sanitize/resolve-blobrefs/diff-sort-key/is-loop-path/spec-named-paths/classify-rows/total/cap-changed-lines/checks-section/negative-path-transcript-pin/fail-open-fallback-pin/quoting-hint-pin verified"

# Sourcing evidence.nu imports its `def main`; nu auto-invokes it after
# the top level runs — exit explicitly so the smoke never reaches it.
exit 0
