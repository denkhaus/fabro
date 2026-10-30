#!/usr/bin/env nu
# Capture review evidence for the reviewer (read-only by capability and
# policy since fabro-1dae; tools available for verification).
#
# PIPE (agent-reviewer era, run 01M0T2GW): the reviewer is an agent node
# with real tools, and the engine materializes demoted values as blob
# files in the sandbox (.fabro/blobs/<sha>.json) that the reviewer reads
# on demand. Consequence: this capture is COMPLETE — it does not
# self-budget. The old 6.8k self-budget was a workaround for the
# prompt-node reviewer (tool_time_ms 0, blob refs unreadable) and it cut
# exactly the wrong files: run 01M0T2GW reviewer@3 journaled that the
# budget walk kept README.md and omitted main.go + fib_test.go — the
# files the review hinged on.
#
# Size handling is the ENGINE's business now: a large capture is demoted
# to a preview-plus-blobref marker, and the reviewer follows the blobref
# link (its prompt says so). Completeness is THIS script's business:
# every seed-work diff is included, source files before docs, the seed
# spec untruncated. Inputs that arrive as blob refs are resolved inline
# (resolve-blobrefs); unresolvable refs pass through as links rather
# than being dropped.
#
# HARD_CAP is a pathological-input safety only (a generated 50MB file in
# the diff must not produce an unbounded blob): on cap the disclosure
# names what was cut, and source-first ordering guarantees docs go first.
#
# Structure: private helpers resolve the facts (run base, numstat
# grouping, in-progress seed), section printers render one evidence
# section each, and `main` wires them in the critical-first order:
# integrity header, per-file stat map (completeness checklist,
# fabro-b798), spec, checks, then the diffs.
# Output text is contract: the reviewer prompt names these sections —
# change wording there too.

# Dev-loop machinery this repo owns; never part of a seed's review scope.
const LOOP_PREFIXES = [".fabro/" ".mulch/" ".seeds/" ".prime/" ".agents/" "docs/" "scripts/"]
const LOOP_ROOTS = ["justfile" "AGENTS.md" "CLAUDE.md" ".gitignore"]

# Pathological-input safety only — NOT a fidelity budget (see header).
const HARD_CAP = 128000

# Doc-ish extensions sort LAST in the diff walk: review hinges on source.
const DOC_EXTENSIONS = ["md" "markdown" "txt" "rst" "adoc" "ad"]

# Loop-churn/anomaly files render numstat plus at most this many CHANGED
# diff lines each (fabro-020b): full churn diffs once pushed captures past
# the reviewer's inline ceiling (.seeds hunks embed multi-KB seed bodies),
# forcing a per-cycle blob-paging detour. Seed-work diffs stay complete.
const CHURN_DIFF_LINE_CAP = 30

# ---------------------------------------------------------------------------
# helpers — resolve facts
# ---------------------------------------------------------------------------

# Pipe-safe emission (fabro-ad23): a single `print` of a string larger than
# the pipe buffer (64KiB on Linux) can hit EAGAIN ("Operation would block")
# when the stage reader lags — the 01M1VZTJSZ5 run died exactly there
# printing a 100KiB+ diff. Emit in line-aligned chunks well under the pipe
# capacity so every write fits the buffer; over-long single lines are split
# at character boundaries (nu substring is codepoint-safe, UTF-8 survives).
def emit [text: string]: nothing -> nothing {
    const EMIT_BUDGET = 30000
    if ($text | str length) <= $EMIT_BUDGET {
        print $text
        return
    }
    mut buf = ""
    for line in ($text | lines) {
        mut piece = $line
        # Pathological single line (minified blob): split by characters.
        while ($piece | str length) > $EMIT_BUDGET {
            print --no-newline ($piece | str substring 0..<$EMIT_BUDGET)
            $piece = ($piece | str substring ($EMIT_BUDGET)..)
        }
        if ($buf | str length) + ($piece | str length) + 1 > $EMIT_BUDGET {
            print $buf
            $buf = ""
        }
        $buf = $buf + $piece + "
"
    }
    if ($buf | is-not-empty) {
        print --no-newline $buf
    }
}

# Wrap bare " /word " tokens in backticks: agent nodes treat them as skill
# references ("Unknown skill" crash). Two passes catch consecutive tokens
# (a /b /c d) — the trailing space of match one is the leading of match two.
def sanitize [text: string]: nothing -> string {
    let one = ($text | str replace --all --regex '(?m)(^|\s)/([a-z][a-z0-9_.-]*)(\s|$)' '$1`/$2`$3')
    $one | str replace --all --regex '(?m)(^|\s)/([a-z][a-z0-9_.-]*)(\s|$)' '$1`/$2`$3'
}

# Resolve `blob://sha256/<hex>` markers in text inputs against the
# engine-materialized blob store (.fabro/blobs/<sha>.json in the sandbox
# workdir). A resolvable ref is inlined verbatim — the capture must carry
# the CONTENT it judges by, not a dead marker. An unresolvable ref (blob
# not materialized here) passes through as the blobref link unchanged:
# downstream (the agent reviewer) can still open it; dropping it would
# lose the only pointer.
def resolve-blobrefs [text: string]: nothing -> string {
    let refs = ($text | parse --regex 'blob://sha256/(?P<sha>[0-9a-f]{64})')
    if ($refs | is-empty) {
        return $text
    }
    mut out = $text
    for sha in ($refs | get sha | uniq) {
        let path = $".fabro/blobs/($sha).json"
        if ($path | path exists) {
            let content = (sanitize (open --raw $path | str trim -r -c "\n"))
            $out = ($out | str replace --all $"blob://sha256/($sha)" $content)
        }
    }
    $out
}

# Source files sort before docs in the diff walk (reviewer@3 painpoint,
# run 01M0T2GW): the complete diff of changed SOURCE files is the primary
# review artifact; docs are context. Stable within each group by path.
def diff-sort-key [path: string]: nothing -> string {
    let ext = ($path | path parse | get extension? | default "" | str lowercase)
    if $ext in $DOC_EXTENSIONS { $"z:($path)" } else { $"a:($path)" }
}

def current-branch []: nothing -> string {
    git branch --show-current | str trim
}

# Run id parsed from the run branch (`fabro/run/<id>`); "local" off a
# run branch. Shared by run-base and the checks-transcript path below;
# mirrored in check-transcript.nu — change both together.
def current-run-id []: nothing -> string {
    let id = (current-branch | parse --regex 'fabro/run/(?P<id>[^/]+)$' | get -o id.0 | default '')
    if ($id | is-empty) { "local" } else { $id }
}

# Parent of THIS run's oldest checkpoint commit, identified via the run
# branch name (fabro/run/<id> -> subject "fabro(<id>): ..."). The subject
# match is FIXED-STRING: a regex like "fabro.<id>:" silently matches
# nothing (it lacks the ")" before ":"), which once produced empty diff
# sections and misleading evidence. Foreign run checkpoints merged into
# history are ignored. Fallback: HEAD.
def run-base []: nothing -> record<base: string, short: string, grounded: bool> {
    let run_id = (current-run-id)
    let subject_mark = $"fabro\(($run_id)\):"
    let checkpoints = (git log --format=%H --fixed-strings --grep $subject_mark | lines | compact)
    if ($checkpoints | is-empty) {
        {base: "HEAD", short: (git rev-parse --short HEAD), grounded: false}
    } else {
        let base = (git rev-parse $"($checkpoints | last)^")
        {base: $base, short: (git rev-parse --short $base), grounded: true}
    }
}

def is-loop-path [path: string]: nothing -> bool {
    ($LOOP_PREFIXES | any {|p| $path | str starts-with $p}) or ($path in $LOOP_ROOTS)
}

# Loop-lane classification (fabro-70b5): in the LOOP lane the loop
# assets ARE the seed work (the review scope), and everything else is
# out-of-lane churn for the anomaly section. The set mirrors the loop
# implementer's x.fs_write plus the tracker file: .fabro/**, scripts/**,
# justfile (the .seeds tracker paths left the set with denkhaus/work#990).
# Keep in sync with the loop graph's envelope (the loop graph-contract
# smoke pins the graph side) and with scripts/qualitygate.nu's
# loop-asset?.
def loop-work-path [path: string]: nothing -> bool {
    # fabro-9973: the fabro-dot snapshot dir is the ONE lib/ exception —
    # keep in sync with qualitygate.nu loop-asset? and the loop graph's
    # x.fs_write (the run-scope fixtures pin the triple).
    if ($path | str starts-with "lib/components/fabro-dot/src/snapshots/") { return true }
    let prefix_hit = ([".fabro/" "scripts/"] | any {|q| $path | str starts-with $q})
    ($prefix_hit) or ($path == "justfile")
}

# Paths literally named by the seed spec (fabro-d76c): a seed body that
# names a loop-path file (e.g. a develop-workflow prompt) as part of its
# change used to see that file flagged in the anomaly section anyway —
# the split was pure path-prefix classification, blind to the spec.
# Extraction is MECHANICAL: path-shaped tokens (contain "/", no spaces)
# pulled from the claimed seed's description; a token ending in "**" or
# "*" names every path under its prefix; the four bare root files match
# as whole words. Fail-open by design: null seed / empty description /
# no tokens -> [] and the lane split degrades to the pre-fabro-d76c
# behavior exactly (never a wrong promotion).
def spec-named-paths [wip: any]: nothing -> list<string> {
    if $wip == null { return [] }
    let desc = ($wip | get -o description | default "")
    if ($desc | is-empty) { return [] }
    let toks = ($desc | parse --regex '(?P<tok>[A-Za-z0-9_.@*-]+(?:/[A-Za-z0-9_.@*-]+)+)' | get -o tok | default [])
    let pathed = ($toks
        | each {|t| $t | str replace -r '^[(`(]+' '' | str replace -r "[).,;:'`]+$" '' }
        | where {|t| ($t | str length) > 1 }
        | uniq)
    let bare_roots = ["justfile" "AGENTS.md" "CLAUDE.md" ".gitignore"]
    let bare = ($bare_roots | where {|b|
        let rx = ('\b(?P<x>' + $b + ')\b')
        ($desc | parse --regex $rx | get -o x | default [] | is-not-empty)
    })
    $pathed | append $bare
}

# Does one spec token name this changed path? Exact match, or — only for
# an explicitly globbed token ("dir/**", "dir/*") — prefix match under
# the glob's directory. A bare directory token ("docs/") never promotes
# its children: only an explicit glob does.
def spec-token-names [tok: string, path: string]: nothing -> bool {
    if $tok == $path { return true }
    if ($tok | str ends-with "*") {
        let pre = ($tok | str replace -r '\*+$' '')
        if ($pre | str ends-with "/") and ($path | str starts-with $pre) { return true }
    }
    false
}

def spec-names-path [spec_paths: list<string>, path: string]: nothing -> bool {
    $spec_paths | any {|tok| spec-token-names $tok $path }
}

# numstat rows {add del path} for base -> working tree (staged + unstaged).
# numstat emits "-" for binary files; `total` counts those as 0.
def numstat-rows [base: string]: nothing -> list<record<add: string, del: string, path: string>> {
    git diff --numstat $base
    | lines
    | compact
    | each {|l| $l | parse --regex '(?P<add>\d+|-)\s+(?P<del>\d+|-)\s+(?P<path>.+)$' | get 0 }
}

def total [rows: list, col: string]: nothing -> int {
    # Guard the empty case: `math sum` errors on an empty stream (seen with
    # a clean worktree and no run base), and numstat "-" counts as 0.
    let vals = ($rows | get -o $col | default [] | each {|x| if $x == '-' { 0 } else { $x | into int } })
    if ($vals | is-empty) { 0 } else { $vals | math sum }
}

def worktree-state []: nothing -> record<lines: list<string>, label: string> {
    let lines = (git status --porcelain | lines | compact)
    let label = (if ($lines | is-empty) { "clean" } else { $"dirty: ($lines | length) entries" })
    {lines: $lines, label: $label}
}

# The claimed item comes from CONTEXT via the engine (stdin_source=
# "current_item_json" on the evidence node) — never from scanning the
# tracker for claims: run 01M294HH captured a DIFFERENT concurrent
# item's spec and diff base that way. The channel resolve and the
# claims check below hard-fail on any mismatch (denkhaus/work#990).

# Resolve the claimed item (denkhaus/work#990): stdin is the
# granted-item packet {itemKey,title,body} the planner forwarded from
# its tool result — the packet NAMES the item, the CHANNEL resolves it
# authoritatively (`fabro work show`) and the run's claim is verified
# against the live claims ledger (`fabro work claims` must show THIS
# run's claim on THAT item — the integrity check packets could never
# do; run 01M294HH's wrong-item capture class dies at the source).
# A malformed packet, an unresolvable item, or a missing claim each
# hard-fail: a silent fallback would reintroduce the wrong-item capture.
def work-server []: nothing -> string {
    let server = ($env.FABRO_SERVER? | default '' | str trim)
    if ($server | is-empty) {
        print -e "evidence: FABRO_SERVER is not set — the work channel needs it (run sandboxes carry it via the channel mount)"
        exit 1
    }
    $server
}

# One outbound work-facade call: the args list carries the subcommand
# and ITS flags verbatim (a rest-param would eat `--project` as THIS
# command's flag), the helper adds the binary and the server. Returns
# the complete-wrapped result record. MIRRORED from closeout.nu —
# reuse this shape, do not invent a new one.
def work-call [args: list<string>]: nothing -> any {
    let bin = ($env.FABRO_BIN? | default 'fabro')
    do { ^$bin work --server (work-server) ...$args } | complete
}

# The live claim states that hold an item (the push gate's live set).
def live-claim-states []: nothing -> list<string> {
    ["granted" "active" "awaiting_merge"]
}

# The capture-time integrity check (denkhaus/work#990): THIS run must
# hold a live claim on EXACTLY this item. Hard-fail otherwise — the
# capture speaks for a claim, not for a key that happens to parse.
def verify-run-claim [item_key: string]: nothing -> nothing {
    let rid = (current-run-id)
    let res = (work-call [claims --project fabro --json])
    if $res.exit_code != 0 {
        print -e $"evidence: work claims failed — cannot verify this run's claim: ($res.stderr | str trim)"
        exit 1
    }
    let claims = (try { $res.stdout | from json } catch { null })
    let claims_kind = ($claims | describe)
    if not (($claims_kind | str starts-with "list") or ($claims_kind | str starts-with "table")) {
        print -e $"evidence: work claims returned unparsable output \(not a claim list\) — cannot verify this run's claim on \($item_key\)"
        exit 1
    }
    let held = ($claims | where {|c|
        (($c | get -o itemKey | default '') == $item_key) and (
            ($c | get -o runId | default '') == $rid
        ) and (
            ($c | get -o state | default '') in (live-claim-states)
        )
    })
    if ($held | is-empty) {
        print -e $"evidence: run \($rid\) holds no live claim on \($item_key\) — the capture would speak for an item this run is not working"
        exit 1
    }
}

# Resolve the claimed item: packet for the key, channel for the truth.
def claimed-item [work_item: bool]: nothing -> any {
    if not $work_item {
        print -e "evidence: --work-item is the only mode (the seeds stdin leg died with the tracker)"
        exit 2
    }
    let raw = (cat | str join | str trim)
    if ($raw | is-empty) {
        print -e "evidence: stdin carried no granted-item packet (current_item_json missing — planner context_updates drop)"
        exit 1
    }
    let pkt = (try { $raw | from json } catch { null })
    if not ($pkt | describe | str starts-with "record") {
        print -e $"evidence: granted-item packet is not a JSON object: ($raw | str substring 0..120)"
        exit 1
    }
    let key = ($pkt | get -o itemKey | default "" | str trim)
    if ($key | is-empty) {
        print -e "evidence: granted-item packet carries no itemKey"
        exit 1
    }
    verify-run-claim $key
    let res = (work-call [show --project fabro $key --json])
    if $res.exit_code != 0 {
        print -e $"evidence: work show \($key\) failed — item not resolvable through the channel: ($res.stderr | str trim)"
        exit 1
    }
    let item = (try { $res.stdout | from json } catch { null })
    if not ($item | describe | str starts-with "record") or (($item | get -o itemKey | default '') != $key) {
        print -e $"evidence: work show \($key\) returned no matching item record"
        exit 1
    }
    {id: $key, title: ($item | get -o title | default ""), description: ($item | get -o body | default "")}
}

# Diff anchor: the run base. Work-item claims are server-side
# (ADR-0032): no tracker-file claim transition exists to anchor on, so
# the capture diffs against the run base directly — the former
# .seeds/issues.jsonl claim walk could never match a work-item key and
# only ever hit this run-base path (denkhaus/work#1056).

# ---------------------------------------------------------------------------
# section printers — one evidence section each (output text is contract)
# ---------------------------------------------------------------------------

def integrity-section [
    base_short: string
    seed_desc: string
    diff_desc: string
    seed_rows: list
    churn_rows: list
    wt_label: string
]: nothing -> string {
    $"(integrity-line $base_short $seed_desc $diff_desc)\n(integrity-line-counts $seed_rows $churn_rows $wt_label)\n"
}

# The header line naming both bases mechanically: the run base (numstat
# and integrity counts stay run-scoped) and the diff base the diff
# section is anchored at (the run base — claims are server-side,
# denkhaus/work#1056). Reprinted at the end so it survives a
# tail-anchored truncation.
def integrity-line [
    base_short: string
    seed_desc: string
    diff_desc: string
]: nothing -> string {
    $"evidence: base=($base_short) seed=($seed_desc) diff-base=($diff_desc)"
}

def integrity-line-counts [
    seed_rows: list
    churn_rows: list
    wt_label: string
]: nothing -> string {
    let seed_files = ($seed_rows | get -o path | default [])
    let churn_files = ($churn_rows | get -o path | default [])
    $"integrity: seed-work=($seed_files | length) files +(total $seed_rows add)/-(total $seed_rows del) | loop-churn=($churn_files | length) files +(total $churn_rows add)/-(total $churn_rows del) | worktree=($wt_label)"
}

# Per-file plus/minus stat map at the TOP of the capture (fabro-b798):
# the reviewer's completeness checklist before paging into (or
# blob-reading) anything below. One line per changed file since the run
# base, run-scoped like the integrity counts — seed work first, loop
# churn second, each tagged so the reviewer can cross-check every line
# against the sections that follow (seed-work diff, loop-work diff,
# anomaly) and see a hard-cap omission against this map without
# reassembling the full capture. Pure: rows in, text out.
def stat-map-section [seed_rows: list, churn_rows: list]: nothing -> string {
    let head = "\n== stat map: every changed file since run base, +adds/-dels (completeness checklist — every file listed here must appear in a section below; seed = seed work, churn = loop churn; a `hard cap hit` omission below is visible against this map) ==\n"
    if ($seed_rows | is-empty) and ($churn_rows | is-empty) {
        return ($head + "(no changed files since run base)\n")
    }
    let seed_lines = ($seed_rows | each {|r| $"seed  +($r.add)/-($r.del) ($r.path)" })
    let churn_lines = ($churn_rows | each {|r| $"churn +($r.add)/-($r.del) ($r.path)" })
    $head + ($seed_lines | append $churn_lines | str join "\n") + "\n"
}

def spec-section [wip: any]: nothing -> string {
    if $wip == null { return "" }
    let head = "\n== in-progress seed spec (authoritative — judge against this, not the brief) ==\n"
    # COMPLETE, untruncated, blob refs resolved: a cut spec once forced
    # "judge the visible part only" reviews; the engine blob-demotion
    # makes truncation unnecessary (see header).
    let desc = (resolve-blobrefs ($wip.description | str trim -r -c "\n"))
    $head + $desc + "\n"
}

def seed-work-files-section [seed_rows: list]: nothing -> string {
    let head = "\n== seed work: changed files (review scope — complete diff below) ==\n"
    if ($seed_rows | is-empty) {
        $head + "(none — no project source changed since run base)\n"
    } else {
        let lines = ($seed_rows | each {|r| $"($r.path) +($r.add)/-($r.del)" } | str join "\n")
        $"($head)($lines)\n"
    }
}

# First line of a git command's stderr, sanitized, for error disclosure
# (fabro-41fa). Empty string when stderr is empty — the note stays, the
# hint just goes quiet.
def first-stderr-line [stderr: string]: nothing -> string {
    let line = ($stderr | lines | where {|l| ($l | str trim) != "" } | get -o 0 | default "")
    sanitize $line
}

# Record of one LISTED file whose `git diff` exited non-zero (fabro-41fa):
# path, exit code, sanitized first stderr line. Pure data; the body note
# and the section disclosure are rendered from it.
def failed-diff-record [path: string, res: record]: nothing -> record<path: string, exit: int, err: string> {
    {path: $path, exit: $res.exit_code, err: (first-stderr-line $res.stderr)}
}

# Shared section-level disclosure for failed diffs (fabro-41fa): names
# every file whose diff errored so a capture failure is an ERROR, never
# an UNSEEN/omitted file. Pure: failed records in, note string or null
# (nothing failed) out — pinned in evidence-smoke.
def walk-failure-note [failed: list]: nothing -> any {
    if ($failed | is-empty) { return null }
    let names = ($failed | each {|f| $"($f.path) \(code ($f.exit)\)" } | str join ", ")
    "\n(git diff FAILED for " + ($failed | length | into string) + " listed file(s) — " + $names + ": capture ERROR, these files are NOT reviewed and NOT merely omitted)"
}

# Section-level disclosure for listed files with zero per-seed hunks
# (fabro-e042): the file list sits under a "complete diff below"
# header, so a listed file with NO body must say why — adjudicate from
# the stat map and the tracker record — and never read as a cap hit
# (run 01M5844: the reviewer burned adjudication effort proving the
# silent skip was intentional). Pure: empty list in, "" out — pinned
# in evidence-smoke.
def zero-hunk-note [empty: list]: nothing -> string {
    if ($empty | is-empty) { return "" }
    "\n(zero per-seed hunks against the diff base: " + ($empty | str join ', ') + " — listed above with no diff body by design; adjudicate the delta from the stat map and the tracker record)\n"
}

# `complete` keeps stdout as one raw string: the list-of-lines split +
# str join round-trip once injected a stray newline mid-line (observed
# once, never reproduced) — raw capture rules that class out.
# COMPLETE, no fidelity budget: every seed-work file is included, whole,
# source files first and docs last (diff-sort-key). The base is the
# run base (claims are server-side, denkhaus/work#1056) — anchored on
# this run's own work, not every closed seed's.
# Files whose per-seed diff is empty render no body; the section names
# them in its zero-hunk note (fabro-e042) instead of a blank line — a
# listed file under a "complete diff below" header must say why it has
# no body, never read as a cap hit. HARD_CAP is a
# pathological-input safety only; on cap the walk stops and the
# disclosure names the omitted files — with docs sorted last, a cap hit
# eats documentation before source.
# Shared per-seed diff walk behind the seed-work diff section (full
# fidelity — seed work is never capped): diff each file against the
# diff base, sanitize the body, skip files with no hunks (named in the
# zero-hunk note, fabro-e042), stop
# at the HARD_CAP and report what was cut. The churn sections use
# capped-diff-walk (fabro-020b) instead.
# A non-zero `git diff` exit on a LISTED file is a capture ERROR, never
# an omission (fabro-41fa): the walk names the file in the body with its
# exit code and first stderr line, counts that note toward the cap, and
# keeps the file OUT of `omitted` — a failed diff must surface as an
# error the reviewer can see, not as an UNSEEN file indistinguishable
# from a cap hit. Same contract in capped-diff-walk below; the shared
# section disclosure is walk-failure-note.
def diff-walk [base: string, files: list]: nothing -> record<body: string, omitted: list, failed: list, empty: list> {
    mut used = 0
    mut parts = []
    mut included = []
    mut failed = []
    mut empty = []
    for f in $files {
        let res = (do { git diff -U3 $base -- $f } | complete)
        $included = ($included | append $f)
        if $res.exit_code != 0 {
            $failed = ($failed | append (failed-diff-record $f $res))
            let note = $"($f): git diff exited ($res.exit_code) — capture ERROR, file NOT reviewed\ndevice output: (first-stderr-line $res.stderr)"
            let cost = ($note | str length)
            if ($used + $cost) > $HARD_CAP { break }
            $used = ($used + $cost)
            $parts = ($parts | append $"($note)\n")
            continue
        }
        let text = (sanitize ($res.stdout | str trim -r -c "\n"))
        if ($text | is-empty) {
            $empty = ($empty | append $f)
            continue
        }
        let cost = ($text | str length)
        if ($used + $cost) > $HARD_CAP { break }
        $used = ($used + $cost)
        $parts = ($parts | append $"($text)\n")
    }
    {body: ($parts | str join), omitted: ($files | where {|f| $f not-in $included }), failed: $failed, empty: $empty}
}

# A CHANGED line of a unified diff: starts with + or - but is not a
# ---/+++ file header. Pure predicate shared by cap-changed-lines.
def is-changed-line [line: string]: nothing -> bool {
    (($line | str starts-with "+") and (not ($line | str starts-with "+++"))) or (($line | str starts-with "-") and (not ($line | str starts-with "---")))
}

# Cap a single-file unified diff to its first `cap` CHANGED lines
# (fabro-020b): file headers, hunk headers, and context lines inside the
# kept region pass through unchanged; at the first changed line past the
# cap the walk stops and a disclosure line names how many changed lines
# were cut (the reviewer adjudicates from the file list + numstat and
# reads the rest with tools). Pure: text in, record out.
def cap-changed-lines [text: string, cap: int]: nothing -> record<body: string, cut: int> {
    mut kept = []
    mut changed = 0
    mut cut = 0
    mut done = false
    for line in ($text | lines) {
        if $done {
            if (is-changed-line $line) { $cut = ($cut + 1) }
            continue
        }
        if (is-changed-line $line) {
            if $changed >= $cap {
                $done = true
                $cut = 1
                continue
            }
            $changed = ($changed + 1)
        }
        $kept = ($kept | append $line)
    }
    if $cut > 0 {
        $kept = ($kept | append $"... truncated: ($cut) further changed lines not shown — churn diffs cap at the first ($cap) changed lines; use tools for the full hunk")
    }
    {body: ($kept | str join "\n"), cut: $cut}
}

# Capped churn walk (fabro-020b): like diff-walk but each file renders as
# its numstat line plus the first CHURN_DIFF_LINE_CAP changed lines of its
# diff — loop-churn files (.seeds hunks embed multi-KB seed bodies) stay
# reviewable without blowing the capture past the reviewer's inline
# ceiling. Same HARD_CAP pathological-input safety and omitted-file
# disclosure as diff-walk; rows (not paths) in, so the numstat line rides
# with the diff. Same failed-diff contract as diff-walk (fabro-41fa): a
# non-zero `git diff` exit on a listed file is a capture ERROR named in
# the body and disclosed by walk-failure-note, never an omission.
def capped-diff-walk [base: string, rows: list]: nothing -> record<body: string, omitted: list, failed: list> {
    mut used = 0
    mut parts = []
    mut included = []
    mut failed = []
    for r in $rows {
        let res = (do { git diff -U3 $base -- $r.path } | complete)
        $included = ($included | append $r.path)
        if $res.exit_code != 0 {
            $failed = ($failed | append (failed-diff-record $r.path $res))
            let note = $"($r.path) +($r.add)/-($r.del): git diff exited ($res.exit_code) — capture ERROR, file NOT reviewed\ndevice output: (first-stderr-line $res.stderr)"
            let cost = ($note | str length)
            if ($used + $cost) > $HARD_CAP { break }
            $used = ($used + $cost)
            $parts = ($parts | append $"($note)\n")
            continue
        }
        let text = (sanitize ($res.stdout | str trim -r -c "\n"))
        if ($text | is-empty) {
            continue
        }
        let capped = (cap-changed-lines $text $CHURN_DIFF_LINE_CAP)
        let block = $"($r.path) +($r.add)/-($r.del)\n($capped.body)"
        let cost = ($block | str length)
        if ($used + $cost) > $HARD_CAP { break }
        $used = ($used + $cost)
        $parts = ($parts | append $"($block)\n")
    }
    {body: ($parts | str join), omitted: ($rows | get -o path | default [] | where {|f| $f not-in $included }), failed: $failed}
}

def diff-section [base: string, seed_rows: list]: nothing -> string {
    let head = "\n== seed work: complete diff (git diff -U3 against the diff base named in the header, files above; source before docs) ==\n"
    let seed_files = ($seed_rows | get -o path | default [] | sort-by {|f| diff-sort-key $f })
    if ($seed_files | is-empty) {
        return ($head + "(no seed-work files to diff)\n")
    }
    let walk = (diff-walk $base $seed_files)
    let failure = (walk-failure-note $walk.failed)
    let zero = (zero-hunk-note $walk.empty)
    if ($walk.omitted | is-empty) {
        $head + $walk.body + $zero + ($failure | default "")
    } else {
        $head + $walk.body + $zero + ($failure | default "") + "\n(hard cap hit: " + ($walk.omitted | length | into string) + " of " + ($seed_files | length | into string) + " files omitted — treat them as UNSEEN and reject on exact grounds if they matter)\n"
    }
}

# Churn-only dev-loop seeds (seed_rows empty, churn_rows not) used to
# capture as seed-work=0 files +0/-0 with no diff at all, forcing reviewers
# into a manual git detour (fabro-4b57). Since fabro-020b the render is
# CAPPED: one numstat line plus the first CHURN_DIFF_LINE_CAP changed
# lines per file — full churn diffs pushed captures past the reviewer's
# inline ceiling every cycle, so the file list carries the adjudication
# contract and tools carry the rest. Same HARD_CAP pathological-input
# safety and disclosure as the seed-work section — only the file set, the
# per-file cap, and the header differ. Emission is gated in `main`: NOT
# emitted when seed_rows is non-empty (loop churn stays counts-only) or
# when both lists are empty.
def loop-diff-section [base: string, churn_rows: list]: nothing -> string {
    let head = "\n== loop work: changed files, numstat + first 30 changed lines each (churn-only seed — capped diff against the diff base named in the header; the FILE LIST is the review scope, source before docs) ==\n"
    let walk = (capped-diff-walk $base $churn_rows)
    let failure = (walk-failure-note $walk.failed)
    if ($walk.omitted | is-empty) and ($walk.body | is-empty) {
        return ($head + ($failure | default "") + "(no loop-work hunks against the diff base)\n")
    }
    if ($walk.omitted | is-empty) {
        $head + $walk.body + ($failure | default "")
    } else {
        $head + $walk.body + ($failure | default "") + "\n(hard cap hit: " + ($walk.omitted | length | into string) + " of " + ($churn_rows | length | into string) + " files omitted — treat them as UNSEEN and reject on exact grounds if they matter)\n"
    }
}

# Mixed captures (seed work AND loop churn, fabro-c0e5) used to demote
# loop paths to counts-only churn, hiding out-of-spec residue from the
# reviewer (PR #109: an auto_merge=false flip across three workflow.toml
# files was approved unseen). This section surfaces every loop-path
# change alongside seed work as a NAMED, DIFFED anomaly — same walk, same
# cap, same disclosure as the two diff sections above — and the reviewer
# prompt makes adjudicating it (residue vs adjacent repair vs scope
# creep) mandatory. Since fabro-d76c the section is spec-aware: loop-path
# files the seed spec explicitly names are seed work (diffed in the
# seed-work sections above), so the anomaly list carries only files the
# spec does NOT name — the header's claim is now literally true.
# Detection-only by design: spec-named path promotion
# (fabro-93a7) and worktree-diff quarantine (fabro-9d2f) are separate
# open seeds and stay out. Since fabro-020b the render is CAPPED: one
# numstat line plus the first CHURN_DIFF_LINE_CAP changed lines per file
# — full loop-churn diffs (.seeds hunks embed multi-KB seed bodies) once
# pushed the capture past the reviewer's inline ceiling, so the
# adjudicate-every-file contract is carried by the FILE LIST plus the
# truncation disclosure; the reviewer reads the rest with tools.
def anomaly-section [base: string, churn_rows: list]: nothing -> string {
    let head = "\n== anomaly: changed files NOT named by the seed spec (loop paths changed alongside seed work, excluding spec-named files — each file numstat + first 30 changed lines only; reviewer must adjudicate EVERY listed file: residue vs adjacent repair vs scope creep) ==\n"
    let walk = (capped-diff-walk $base $churn_rows)
    let failure = (walk-failure-note $walk.failed)
    if ($walk.omitted | is-empty) and ($walk.body | is-empty) {
        return ($head + ($failure | default "") + "(no anomaly hunks against the diff base — loop files changed before the base only)\n")
    }
    if ($walk.omitted | is-empty) {
        $head + $walk.body + ($failure | default "")
    } else {
        $head + $walk.body + ($failure | default "") + "\n(hard cap hit: " + ($walk.omitted | length | into string) + " of " + ($churn_rows | length | into string) + " files omitted — treat them as UNSEEN and reject on exact grounds if they matter)\n"
    }
}

def loop-churn-section [churn_rows: list]: nothing -> string {
    let head = "\n== loop churn (dev-loop machinery: workflow/scripts/tracker/expertise/config; counts only, not seed work) ==\n"
    if ($churn_rows | is-empty) {
        $head + "(none)\n"
    } else {
        let lines = ($churn_rows | each {|r| $"($r.path) +($r.add)/-($r.del)" } | str join "\n")
        $"($head)($lines)\n"
    }
}

def worktree-section [wt_lines: list<string>]: nothing -> string {
    let head = "\n== working tree == git status --porcelain (untracked files show here; they are in NO diff above) ==\n"
    if ($wt_lines | is-empty) {
        $head + "(clean)\n"
    } else {
        let lines = ($wt_lines | each {|l| sanitize $l } | str join "\n")
        $"($head)($lines)\n"
    }
}

# Pure path builder for the per-criterion check transcript written by
# check-transcript.nu (implementer-side half; dir constant kept in sync
# there, including the FABRO_CHECK_TRANSCRIPT_DIR battery override).
# Takes the run id so the smoke can exercise it without git.
def checks-transcript-path [run_id: string]: nothing -> string {
    $"($env.FABRO_CHECK_TRANSCRIPT_DIR? | default '/tmp/fabro-check-transcript')/($run_id).md"
}

# Per-criterion check transcript (fabro-d89a + fabro-f759): checks the
# implementer (implement runs) or the planner (verification-only claims,
# which skip the implementer entirely) recorded through check-transcript.nu
# — command + combined output + exit code records, each stamped with its
# recorder (`[by implementer]` / `[by planner]`) — append to the
# run-scoped transcript; this section inlines it so the reviewer approves
# from context instead of re-running the proof (temp fixtures may be long
# gone — the transcript IS the proof). A verification-only capture with
# empty seed-work is exactly the case that needs it most: without it the
# capture is a bare "(no seed-work files to diff)" and the reviewer
# re-derives every criterion with tools. Present in EVERY capture whose
# running stage recorded checks; omitted entirely when no transcript
# exists (no empty-section noise for runs that recorded none). HARD_CAP
# applies with the same UNSEEN disclosure as the diff sections.
def checks-section [path: string]: nothing -> string {
    if not ($path | path exists) { return "" }
    let raw = (open --raw $path | str trim -r -c "\n")
    if ($raw | is-empty) { return "" }
    let head = "\n== recorded checks: per-criterion transcript (commands + outputs + exit codes exactly as recorded through the check-transcript wrapper — implementer on implement runs, planner on verification-only runs; recorded proof; never re-run these; when a command appears in multiple records the LATEST record is the proof of record — re-runs are stamped `[re-run N]` and supersede earlier attempts) ==\n"
    let body = (sanitize $raw)
    if (($head | str length) + ($body | str length)) > $HARD_CAP {
        let keep = ($HARD_CAP - ($head | str length) - 100)
        $head + ($body | str substring 0..<$keep) + "\n(hard cap hit: transcript cut — the cut tail is UNSEEN; treat with the same rule as omitted diff files)\n"
    } else {
        $head + $body + "\n"
    }
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

# Pure classification (fabro-70b5 lane split + fabro-d76c spec-awareness):
# product lane inverts loop-ness, loop lane inverts it again (loop assets
# = seed work); the tracker file flips sides with the lane (product churn
# vs loop seed work — reviewed hunks). A spec-named path flips to seed
# work regardless of lane, so the anomaly section carries only files the
# seed spec does NOT name. Pure on purpose: the smoke proves the split
# without git.
def classify-rows [rows: list, lane: string, spec_paths: list<string>]: nothing -> record<seed: list, churn: list> {
    let spec_named = {|r| spec-names-path $spec_paths $r.path }
    let seed = (if $lane == "loop" {
        $rows | where {|r| (loop-work-path $r.path) or (do $spec_named $r)} | sort-by path
    } else {
        $rows | where {|r| (not (is-loop-path $r.path)) or (do $spec_named $r)} | sort-by path
    })
    let churn = (if $lane == "loop" {
        $rows | where {|r| (not (loop-work-path $r.path)) and (not (do $spec_named $r))} | sort-by path
    } else {
        $rows | where {|r| (is-loop-path $r.path) and (not (do $spec_named $r))} | sort-by path
    })
    {seed: $seed, churn: $churn}
}

# Lane parameter (fabro-70b5): product (default — develop's reviewer
# scope is repo code, loop paths are churn) or loop (the meta lane's
# reviewer scope IS the loop assets; everything else is out-of-lane and
# surfaces in the anomaly section for adjudication). Passed by the
# calling graph's script line; deterministic via the run's workflow spec.
def main [--lane: string = "product", --work-item]: nothing -> nothing {
    let base = (run-base)
    let no_base_note = (if not $base.grounded {
        "NO RUN BASE — no checkpoint commits found for this run.\nThe diff below is empty or misleading; treat this evidence as unreliable.\n\n"
    } else { "" })

    # Journal files (.fabro/journal/<run-id>.jsonl, one line per stage
    # completion, plus legacy <node>@<visit>.json) are meta context:
    # run-machinery stage records about the workflow itself, consumed later
    # by the improve workflow. They are NO review input —
    # dropped here, so they appear in NO section and NO count.
    let rows = (numstat-rows $base.base | where {|r| not ($r.path | str starts-with ".fabro/journal/")})
    # The claimed seed FIRST: its description names the files the spec
    # mandates, and spec-named files are seed work in EVERY lane
    # (fabro-d76c) — a spec that names a loop-path file (a develop
    # workflow prompt, say) must not see it flagged as anomaly churn.
    let wip = (claimed-item $work_item)
    let split = (classify-rows $rows $lane (spec-named-paths $wip))
    let seed_rows = $split.seed
    let churn_rows = $split.churn

    let wt = (worktree-state)
    let seed_desc = (if $wip == null { "none-in-progress" } else { $"($wip.id): ($wip.title)" })

    # Diff anchor: the run base. Only the diff scope changes — numstat
    # rows, integrity counts, and the file list above stay run-scoped
    # per spec (they are small). ADR-0032: work-item claims are
    # server-side — no tracker-file claim transition exists to anchor
    # on, so the capture diffs against the run base directly (flagged
    # in the header, never silent).
    let diff_desc = ($base.short + " (run base — claims are server-side, no tracker-file claim anchor)")

    # Fixed sections first, each under its own cap. The stat map rides
    # directly under the integrity header (fabro-b798): a tail-truncated
    # or blob-demoted capture still opens with the completeness map.
    let integrity = (integrity-section $base.short $seed_desc $diff_desc $seed_rows $churn_rows $wt.label)
    let stat_map = (stat-map-section $seed_rows $churn_rows)
    let spec = (spec-section $wip)
    # Implementer check transcript (fabro-d89a; planner-recording on
    # verification-only claims fabro-f759): cheap and critical —
    # slot it after the spec, before the file lists and diffs, so a
    # compact render still carries the recorded proof.
    let checks = (checks-section (checks-transcript-path (current-run-id)))
    let files = (seed-work-files-section $seed_rows)
    let churn = (loop-churn-section $churn_rows)
    # Churn-only seeds get a loop-work diff adjacent to the churn counts
    # (fabro-4b57); mixed or clean captures are unchanged.
    let loop_diff = (if ($seed_rows | is-empty) and (not ($churn_rows | is-empty)) {
        loop-diff-section $base.base $churn_rows
    } else { "" })
    # Mixed captures (seed work AND loop churn) get an anomaly section
    # (fabro-c0e5): loop paths changed alongside seed work are diffed,
    # not demoted to the counts-only churn above.
    let anomaly = (if (not ($seed_rows | is-empty)) and (not ($churn_rows | is-empty)) {
        anomaly-section $base.base $churn_rows
    } else { "" })
    let worktree = (worktree-section $wt.lines)

    # …then the diff gets whatever remains under the hard output budget
    # (tail sections reserved so a cut lands in the diff, never in them).
    let diff = (diff-section $base.base $seed_rows)

    # Critical-first emission. No size games: a large capture is the
    # engine's to demote (blobref link) and the agent reviewer's to read.
    emit ($no_base_note | str trim -r -c "\n")
    emit $integrity
    emit $stat_map
    emit $spec
    if ($checks | is-not-empty) { emit $checks }
    emit $files
    emit $diff
    emit $churn
    if ($loop_diff | is-not-empty) { emit $loop_diff }
    if ($anomaly | is-not-empty) { emit $anomaly }
    emit $worktree
    # Duplicate of the header line: survives a tail-anchored truncation too.
    print ""
    print (integrity-line $base.short $seed_desc $diff_desc)
    print "== evidence complete =="
}
