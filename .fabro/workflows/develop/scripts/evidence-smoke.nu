#!/usr/bin/env nu
# Smoke test for evidence.nu's PURE helpers (fabro-bfe1): sanitize,
# resolve-blobrefs, diff-sort-key, is-loop-path, total — exercised over
# canned fixtures without git (helpers that shell out — numstat-rows,
# run-base, worktree-state, claimed-seed — are out of scope here;
# their logic is covered by the parse check in scripts/qualitygate.nu
# and manual invocation). The `source` const resolves against THIS
# file's directory, so the script runs from any cwd (closeout-smoke
# pattern):
#   nu .fabro/workflows/develop/scripts/evidence-smoke.nu

const EVIDENCE = "evidence.nu"
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

print "evidence-smoke: ok — sanitize/resolve-blobrefs/diff-sort-key/is-loop-path/spec-named-paths/classify-rows/total/checks-section verified"

# Sourcing evidence.nu imports its `def main`; nu auto-invokes it after
# the top level runs — exit explicitly so the smoke never reaches it.
exit 0
