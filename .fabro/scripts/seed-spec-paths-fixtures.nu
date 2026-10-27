#!/usr/bin/env nu
# prompt-lint: synthetic-seed-ids
# Seed-spec path fixtures (fabro-8a60): the pure core of the
# check-seed-spec-paths tier — spec-named-loop-paths extraction,
# spec-paths-missing, claimed-item-for-run — over decisive canned
# shapes, plus the end-to-end RED/GREEN arms of the subcommand over a
# scratch git fixture (run branch + checkpoint subject, the
# run-scope-fixtures pattern). The tier's purpose is the incident
# class: an empty-edit or wrong-file pass consuming a full review
# cycle; these fixtures prove the RED fires exactly there and nowhere
# else.
#
#   nu .fabro/scripts/seed-spec-paths-fixtures.nu

# `source` resolves relative to THIS file's directory (anchor_check.nu
# idiom); the runtime path for re-invocation is derived at run time.
source ../../scripts/qualitygate.nu
const SCRIPT_DIR = (path self | path dirname)
let QUALITYGATE = ($SCRIPT_DIR | path join '../..' 'scripts' 'qualitygate.nu' | path expand)

def fail [what: string]: nothing -> nothing {
    print -e $"seed-spec-paths-fixtures: FAIL — ($what)"
    exit 1
}

def expect-extraction [desc: string, expected: list<string>]: nothing -> nothing {
    let got = (spec-named-loop-paths $desc)
    if $got != $expected {
        fail $"extraction: expected \($expected | to json -r\), got \($got | to json -r\)"
    }
}

# ── extraction: what counts as a named loop-asset path ────────────────
# The motivating shape: a spec naming the exact files it wants changed.
expect-extraction "In `scripts/qualitygate.nu`, extend the loop-asset tier. Also edit .fabro/workflows/loop/scripts/loop-gate.nu and .fabro/scripts/battery-runner.nu." [
    "scripts/qualitygate.nu"
    ".fabro/workflows/loop/scripts/loop-gate.nu"
    ".fabro/scripts/battery-runner.nu"
]
# Globs and bare directory prefixes are dropped: the diff lists files,
# a glob or prefix can never match one — keeping them would false-RED
# every spec that names a root sweep (`.fabro/**`). (The historical
# example cited .seeds/issues.jsonl — a loop path until denkhaus/work#990
# swept the tracker out of the envelope.)
expect-extraction "Sweep `.fabro/**` and `scripts/` plus .fabro/scripts/stage-journal.nu" [
    ".fabro/scripts/stage-journal.nu"
]
# fabro-48f3: a bare DIRECTORY token drops even without a trailing
# slash — the prefix rule only caught the `scripts/` spelling, and a
# directory can never appear in a file-only `git diff --name-only`,
# so keeping it was a permanent false RED (the d83a pre-claim body
# rewrite, run 01M5844). Resolved against this worktree; the battery
# runs from the checkout root.
expect-extraction "Update .fabro/workflows and scripts/qualitygate.nu for the tier." [
    "scripts/qualitygate.nu"
]
# fabro-48f3: creation-named paths drop — the claim gate's shared
# creation-intent window (anchor_check.nu) judges the same spec body,
# so `Add scripts/new-probe.nu` is a proposed target, not an edit
# mandate, while `extend scripts/qualitygate.nu` stays enforced.
expect-extraction "Add scripts/new-probe.nu and extend scripts/qualitygate.nu." [
    "scripts/qualitygate.nu"
]
# Out-of-lane slash-tokens stay out: lib/, docs/, apps/ paths are not
# loop-asset surfaces (and the one lib/ snapshot exception IS in).
expect-extraction "Fix `lib/apps/fabro-cli/src/main.rs`, docs/public/x.md, apps/web/y.ts, but lib/components/fabro-dot/src/snapshots/checked_in_workflows.snap is derived" [
    "lib/components/fabro-dot/src/snapshots/checked_in_workflows.snap"
]
# URLs never parse as paths; a bare `justfile` in prose is a word, not
# a path citation (documented tier scope); path:line anchors strip the
# line; a trailing sentence dot does not ride the token.
expect-extraction "See https://host.io/scripts/x.nu and the justfile, then edit `.fabro/scripts/stage-journal.nu:12`." [
    ".fabro/scripts/stage-journal.nu"
]
# fabro-6b0f: journal citations in a spec body are PROVENANCE (the
# Origin/Basis lines point at the reviewer run that filed a residual),
# never edit mandates — the journal is append-only stage records no
# pass edits, so enforcing the citation was a permanent false RED on
# every residual seed.
expect-extraction "Residual from run X. Origin: closed seed fabro-abcd, reviewer journal .fabro/journal/01M59JSC3MKGRCH5PMR53XHADZ.jsonl. Basis: run 01M59JSC3MKGRCH5PMR53XHADZ. Also fix .fabro/scripts/prompt-lint.nu." [
    ".fabro/scripts/prompt-lint.nu"
]
# Revision-record citations (the Basis line pointing at the reviewer's
# revision document) are provenance exactly like journal citations —
# append-only records no pass edits, so enforcing the citation was a
# permanent false RED on every revision-filed seed.
expect-extraction "Revisor pass filed a revision. Basis: revisor revision .fabro/revisions/01M5CVG6XT9ABZFTMGY45QGCKT.md finding 2. Also edit .fabro/workflows/develop/scripts/claim-check.nu." [
    ".fabro/workflows/develop/scripts/claim-check.nu"
]
# An empty/absent description names nothing.
expect-extraction "" []

# ── extraction: brace groups expand BEFORE the path filter ────────────
# (denkhaus/work#6) The motivating incident: a spec citing a comma
# brace group emitted exactly ONE path because the path-character class
# cannot tokenize `{a,b,c}` — the one-unit gate under-constrained the
# whole run. Expansion runs first; every alternative then faces the
# SAME filters as a literal citation (these three exist as files, so
# the directory filter keeps them).
expect-extraction "Rewrite .fabro/workflows/develop/prompts/{planner,implementer,reviewer}.md as one unit." [
    ".fabro/workflows/develop/prompts/planner.md"
    ".fabro/workflows/develop/prompts/implementer.md"
    ".fabro/workflows/develop/prompts/reviewer.md"
]
# Brace citation mid-sentence: trailing sentence punctuation (`,`)
# does not ride the expanded paths; a literal citation alongside still
# extracts independently (literal citations list before expanded ones —
# the pipeline's plain-then-braced order).
expect-extraction "Update .fabro/workflows/loop/prompts/{planner,closeout}.md, then scripts/qualitygate.nu." [
    "scripts/qualitygate.nu" ".fabro/workflows/loop/prompts/planner.md" ".fabro/workflows/loop/prompts/closeout.md"
]
# Negative: a NON-comma brace (`{record}`) is not an enumeration — no
# expansion, and the untokenizable citation drops instead of emitting a
# mangled partial path.
expect-extraction "Fix the .fabro/prompts/{record}.md citation and edit .fabro/scripts/prompt-lint.nu." [
    ".fabro/scripts/prompt-lint.nu"
]
# Negative: creation-intent through the brace literal — an `Add`
# window in front of the brace citation drops EVERY alternative (the
# whole group is the proposed target), not just literally-named ones;
# a literal `extend` citation stays enforced.
expect-extraction "Add .fabro/scripts/{new-probe,other-probe}.nu and extend scripts/qualitygate.nu." [
    "scripts/qualitygate.nu"
]
# Negative: out-of-lane alternatives stay out — expansion does not
# widen the loop-asset allow-list (lib/ and docs/ paths drop exactly as
# literal citations do).
expect-extraction "Rewrite lib/{a,b}.rs and docs/{x,y}.md plus .fabro/scripts/lib/spec_paths.nu." [
    ".fabro/scripts/lib/spec_paths.nu"
]

# ── missing: the RED set ───────────────────────────────────────────────
if (spec-paths-missing ["scripts/qualitygate.nu"] []) != ["scripts/qualitygate.nu"] {
    fail "empty diff must miss every named path — the tier's reason to exist"
}
if (spec-paths-missing ["scripts/qualitygate.nu" ".fabro/x.nu"] ["scripts/qualitygate.nu" "other/file"]) != [".fabro/x.nu"] {
    fail "partial diff must miss exactly the untouched named path"
}
if (spec-paths-missing ["scripts/qualitygate.nu"] ["scripts/qualitygate.nu"]) != [] {
    fail "fully-touched diff must miss nothing"
}

# ── item resolution: this run's live claim ────────────────────────────
let claims = ([
    {runId: "01JOTHER" itemKey: "denkhaus/work#101"}
    {runId: "01JMINE0" itemKey: "denkhaus/work#102"}
])
if (claimed-item-for-run $claims "01JMINE0") != "denkhaus/work#102" {
    fail "resolution must return the item this run holds a claim on"
}
if (claimed-item-for-run $claims "01JNONE0") != "" {
    fail "a run without a live claim resolves nothing"
}
let ambiguous = ([
    {runId: "01JMINE0" itemKey: "denkhaus/work#102"}
    {runId: "01JMINE0" itemKey: "denkhaus/work#103"}
])
if (claimed-item-for-run $ambiguous "01JMINE0") != "" {
    fail "two claims on one run are ambiguity, not a guess"
}

# ── end-to-end: the subcommand REDs on an empty diff and greens when
# the named path is touched, over a scratch git fixture (run branch +
# checkpoint subject, the run-scope-fixtures pattern) and a canned
# `fabro` work-facade shim planted on PATH (the tracker is not reachable
# from a battery — the fake answers the claims/show calls with the same
# JSON shapes; cutover stage 5, denkhaus/work#988).
let tmp = (mktemp -d -t seed-spec-paths.XXXX)
mkdir $"($tmp)/bin"
(
    "#!/bin/sh
case \"\$*\" in
  *' claims --project fabro --json'*) echo '[{\"claimId\":\"c1\",\"projectId\":\"fabro\",\"itemKey\":\"denkhaus/work#701\",\"runId\":\"FIXTURERUN\",\"lane\":\"loop\",\"state\":\"active\",\"grantedAt\":\"2026-10-26T00:00:00Z\",\"updatedAt\":\"2026-10-26T00:00:00Z\",\"prUrl\":null}]'; exit 0;;
  *' show --project fabro denkhaus/work#701 --json'*) echo '{\"itemKey\":\"denkhaus/work#701\",\"title\":\"fixture\",\"body\":\"Extend scripts/qualitygate.nu and .fabro/workflows/loop/scripts/loop-gate.nu with the seed-spec path tier.\",\"state\":\"open\",\"labels\":[],\"priority\":2,\"url\":\"\",\"updatedAt\":\"2026-10-26T00:00:00Z\"}'; exit 0;;
esac
echo 'fake fabro: unsupported' >&2; exit 1"
    | save $"($tmp)/bin/fabro"
)
^chmod +x $"($tmp)/bin/fabro"
$env.PATH = ($env.PATH | prepend $"($tmp)/bin")
cd $tmp
^git init --quiet .
^git config user.email t@example.tld
^git config user.name t
("readme" | save readme.txt)
^git add .
^git commit --quiet -m init
^git checkout --quiet -b fabro/run/FIXTURERUN
# The run base is the parent of the LAST checkpoint commit; the
# working tree after it IS the run diff.

# RED arm 1: empty diff — both named paths missing.
^git commit --quiet --allow-empty -m "fabro(FIXTURERUN): checkpoint 1"
let red = (do { ^nu $QUALITYGATE check-seed-spec-paths } | complete)
if $red.exit_code == 0 {
    fail $"empty diff must RED the tier, got exit 0: \($red.stdout | str trim | lines | last 3 | str join ' | '\)"
}
if not ($red.stdout | str contains 'scripts/qualitygate.nu — NOT in diff') {
    fail $"RED output must name each missing path: \($red.stdout | str trim\)"
}
print "PASS (empty diff REDs and names both missing paths)"

# RED arm 2: wrong file — one named path touched, the other not.
# (New files must be `git add`ed: run-diff-paths reads
# `git diff --name-only <base>`, which sees tracked files only.)
mkdir .fabro/workflows/loop/scripts
("marker" | save .fabro/workflows/loop/scripts/loop-gate.nu)
^git add .fabro
let red2 = (do { ^nu $QUALITYGATE check-seed-spec-paths } | complete)
if $red2.exit_code == 0 {
    fail "a diff missing any named path must RED the tier"
}
if not ($red2.stdout | str contains 'scripts/qualitygate.nu — NOT in diff') {
    fail "partial RED must name the untouched named path"
}
print "PASS (wrong-file diff REDs, naming only the untouched path)"

# GREEN arm: every named path touched.
mkdir scripts
("marker" | save scripts/qualitygate.nu)
^git add scripts
let green = (do { ^nu $QUALITYGATE check-seed-spec-paths } | complete)
if $green.exit_code != 0 {
    fail $"all-paths-touched diff must stay green: \($green.stdout | str trim | lines | last 3 | str join ' | '\)"
}
if not ($green.stdout | str contains 'seed-spec-paths green') {
    fail "green arm must print the green verdict"
}
print "PASS (all named paths touched -> green)"

# Fail-open arm: the tracker call itself fails -> exit 0, tier skipped.
rm $"($tmp)/bin/fabro"
let fo = (do { ^nu $QUALITYGATE check-seed-spec-paths } | complete)
if $fo.exit_code != 0 {
    fail $"work CLI failure must fail open, got exit \($fo.exit_code\)"
}
if not ($fo.stdout | str contains 'fail-open') {
    fail "fail-open arm must say so"
}
print "PASS (work CLI failure fails open)"
cd /
rm -rf $tmp
print "seed-spec-paths fixtures green"
# Explicit exit: the fail-open arm deliberately runs a failing external
# (the real seeds CLI against a repo with no .seeds store — stderr
# 'fatal: not a git repository'); without this pin nu propagates that
# child's 128 as the script's exit code and the battery runner counts
# a green fixture red.
exit 0
