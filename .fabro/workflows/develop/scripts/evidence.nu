#!/usr/bin/env nu
# Evidence METADATA for the pull reviewer (work#1624, ADR-0051).
#
# The capture era is over: the reviewer pulls the run's actual diffs
# itself — `fabro_run_diffs` enumerates the run's checkpoint diffs and
# `fabro_blob` pages patch content — so this stage emits structured
# metadata only. Five records, no diff text:
#
#   1. the integrity header — run base, claimed item, diff base,
#      seed-work/loop-churn totals, worktree state;
#   2. the stat map — one +adds/-dels line per changed file since the
#      run base, tagged seed/churn: the completeness checklist the
#      reviewer pulls against (the fabro-b798 contract, pull-shaped);
#   3. the classification — the seed-work file list (review scope,
#      source before docs) and the loop-churn counts;
#   4. the anomaly list — loop paths changed alongside seed work that
#      the seed spec does not name (fabro-c0e5/fabro-d76c), numstat
#      only — the reviewer adjudicates each by pulling its patch;
#   5. the worktree state — untracked files, which no checkpoint diff
#      carries.
#
# DELETED with the capture (ADR-0051 deletion contract — the rules that
# compensated for it die with it): the git-diff text walks
# (diff-walk/capped-diff-walk), the inline budgets (HARD_CAP,
# EMIT_BUDGET, CHURN_DIFF_LINE_CAP), the blob-ref inlining
# (resolve-blobrefs), the spec/checks inline sections, and every
# truncation disclosure. Size handling has no in-stage machinery left
# to compensate for.
#
# The claimed item still comes from CONTEXT via the engine
# (stdin_source="current_item_json") and the claim-integrity check
# stays (denkhaus/work#990): the stage names the item it classified
# for, hard-failing on any mismatch.
#
# Output text is contract: the reviewer prompt names these records —
# change wording there too.

# Shared lane-script modules (denkhaus/work#1108): the run-anchor triple
# (branch/run-id/run-base) and the work channel (server/bin resolution,
# the facade call) live in .fabro/scripts/lib/ — one implementation,
# every lane script a consumer.
use ../../../scripts/lib/run_anchor.nu
use ../../../scripts/lib/work_channel.nu
# The loop-lane classifier (fabro-70b5): in the LOOP lane the loop assets
# ARE the seed work (the review scope), everything else is out-of-lane
# churn. The classifier is the shared lib's loop-asset? (work#264: the
# loop-work-path mirror retired - one implementation in lib/spec_paths.nu;
# the run-scope fixtures pin the sourcing).
source ../../../scripts/lib/spec_paths.nu

# Dev-loop machinery this repo owns; never part of a seed's review scope.
const LOOP_PREFIXES = [".fabro/" ".mulch/" ".seeds/" ".prime/" ".agents/" "docs/" "scripts/"]
const LOOP_ROOTS = ["justfile" "AGENTS.md" "CLAUDE.md" ".gitignore"]

# Doc-ish extensions sort LAST in the classification lists: review
# hinges on source, so the reviewer pulls source files first
# (reviewer@3 painpoint, run 01M0T2GW). Stable within each group by path.
const DOC_EXTENSIONS = ["md" "markdown" "txt" "rst" "adoc" "ad"]

# ---------------------------------------------------------------------------
# helpers — resolve facts
# ---------------------------------------------------------------------------

# Wrap bare " /word " tokens in backticks: agent nodes treat them as skill
# references ("Unknown skill" crash). Two passes catch consecutive tokens
# (a /b /c d) — the trailing space of match one is the leading of match two.
def sanitize [text: string]: nothing -> string {
    let one = ($text | str replace --all --regex '(?m)(^|\s)/([a-z][a-z0-9_.-]*)(\s|$)' '$1`/$2`$3')
    $one | str replace --all --regex '(?m)(^|\s)/([a-z][a-z0-9_.-]*)(\s|$)' '$1`/$2`$3'
}

# Source files sort before docs in the classification lists: the
# reviewer pulls the source patches first.
def diff-sort-key [path: string]: nothing -> string {
    let ext = ($path | path parse | get extension? | default "" | str lowercase)
    if $ext in $DOC_EXTENSIONS { $"z:($path)" } else { $"a:($path)" }
}

def is-loop-path [path: string]: nothing -> bool {
    ($LOOP_PREFIXES | any {|p| $path | str starts-with $p}) or ($path in $LOOP_ROOTS)
}

# Paths literally named by the seed spec (fabro-d76c): a seed body that
# names a loop-path file (e.g. a develop-workflow prompt) as part of its
# change used to see that file flagged in the anomaly list anyway —
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
# The live claim states that hold an item (the push gate's live set).
def live-claim-states []: nothing -> list<string> {
    ["granted" "active" "awaiting_merge"]
}

# The metadata-time integrity check (denkhaus/work#990): THIS run must
# hold a live claim on EXACTLY this item. Hard-fail otherwise — the
# metadata speaks for a claim, not for a key that happens to parse.
def verify-run-claim [item_key: string]: nothing -> nothing {
    let rid = (run_anchor run-id 'local')
    let res = (work_channel call 'evidence' [claims --project fabro --json])
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
        print -e $"evidence: run \($rid\) holds no live claim on \($item_key\) — the metadata would speak for an item this run is not working"
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
    let res = (work_channel call 'evidence' [show --project fabro $key --json])
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

# ---------------------------------------------------------------------------
# section printers — one metadata record each (output text is contract)
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
# and integrity counts stay run-scoped) and the diff base the stat map
# is anchored at (the run base — claims are server-side,
# denkhaus/work#1056).
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

# Per-file plus/minus stat map right under the integrity header
# (fabro-b798): the reviewer's completeness checklist and PULL LIST —
# one line per changed file since the run base, run-scoped like the
# integrity counts, seed work first and loop churn second, each tagged.
# Every file on the map must be pulled (fabro_run_diffs, paged via
# fabro_blob) and adjudicated; a file the pull doors cannot produce is
# a finding the reviewer names, never a silent skip. Pure: rows in,
# text out.
def stat-map-section [seed_rows: list, churn_rows: list]: nothing -> string {
    let head = "\n== stat map: every changed file since run base, +adds/-dels (completeness checklist — every file listed here must be pulled and adjudicated; seed = seed work, churn = loop churn) ==\n"
    if ($seed_rows | is-empty) and ($churn_rows | is-empty) {
        return ($head + "(no changed files since run base)\n")
    }
    let seed_lines = ($seed_rows | each {|r| $"seed  +($r.add)/-($r.del) ($r.path)" })
    let churn_lines = ($churn_rows | each {|r| $"churn +($r.add)/-($r.del) ($r.path)" })
    $head + ($seed_lines | append $churn_lines | str join "\n") + "\n"
}

def seed-work-files-section [seed_rows: list]: nothing -> string {
    let head = "\n== seed work: changed files (review scope — pull each file's patch) ==\n"
    if ($seed_rows | is-empty) {
        $head + "(none — no project source changed since run base)\n"
    } else {
        let lines = ($seed_rows
            | sort-by {|r| diff-sort-key $r.path }
            | each {|r| $"($r.path) +($r.add)/-($r.del)" } | str join "\n")
        $"($head)($lines)\n"
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

# Mixed records (seed work AND loop churn, fabro-c0e5) get an anomaly
# LIST: loop paths changed alongside seed work, minus the files the
# seed spec names (fabro-d76c — spec-named loop paths are seed work,
# listed above), so the header's claim is literally true. The list is
# NUMSTAT ONLY (work#1624, ADR-0051): no diff text rides here — the
# reviewer adjudicates EVERY listed file (residue vs adjacent repair vs
# scope creep) by pulling its patch, never from capped capture text.
def anomaly-section [churn_rows: list]: nothing -> string {
    let head = "\n== anomaly: changed files NOT named by the seed spec (loop paths changed alongside seed work, excluding spec-named files — adjudicate EVERY listed file: residue vs adjacent repair vs scope creep; pull each file's patch) ==\n"
    if ($churn_rows | is-empty) {
        return ($head + "(no anomaly files against the run base — loop files changed before the base only)\n")
    }
    let lines = ($churn_rows | each {|r| $"($r.path) +($r.add)/-($r.del)" } | str join "\n")
    $"($head)($lines)\n"
}

def worktree-section [wt_lines: list<string>]: nothing -> string {
    let head = "\n== working tree == git status --porcelain (untracked files show here; they are in NO checkpoint diff — flag any that look like seed work or artifacts) ==\n"
    if ($wt_lines | is-empty) {
        $head + "(clean)\n"
    } else {
        let lines = ($wt_lines | each {|l| sanitize $l } | str join "\n")
        $"($head)($lines)\n"
    }
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

# Pure classification (fabro-70b5 lane split + fabro-d76c spec-awareness):
# product lane inverts loop-ness, loop lane inverts it again (loop assets
# = seed work); the tracker file flips sides with the lane (product churn
# vs loop seed work — reviewed files). A spec-named path flips to seed
# work regardless of lane, so the anomaly list carries only files the
# seed spec does NOT name. Pure on purpose: the smoke proves the split
# without git.
def classify-rows [rows: list, lane: string, spec_paths: list<string>]: nothing -> record<seed: list, churn: list> {
    let spec_named = {|r| spec-names-path $spec_paths $r.path }
    let seed = (if $lane == "loop" {
        $rows | where {|r| (loop-asset? $r.path) or (do $spec_named $r)} | sort-by path
    } else {
        $rows | where {|r| (not (is-loop-path $r.path)) or (do $spec_named $r)} | sort-by path
    })
    let churn = (if $lane == "loop" {
        $rows | where {|r| (not (loop-asset? $r.path)) and (not (do $spec_named $r))} | sort-by path
    } else {
        $rows | where {|r| (is-loop-path $r.path) and (not (do $spec_named $r))} | sort-by path
    })
    {seed: $seed, churn: $churn}
}

# Lane parameter (fabro-70b5): product (default — develop's reviewer
# scope is repo code, loop paths are churn) or loop (the meta lane's
# reviewer scope IS the loop assets; everything else is out-of-lane and
# surfaces in the anomaly list for adjudication). Passed by the
# calling graph's script line; deterministic via the run's workflow spec.
def main [--lane: string = "product", --work-item]: nothing -> nothing {
    let base = (run_anchor run-base)
    let no_base_note = (if not $base.grounded {
        "NO RUN BASE — no checkpoint commits found for this run.\nThe stat map below is empty or misleading; treat this metadata as unreliable.\n\n"
    } else { "" })

    # Journal files (.fabro/journal/<run-id>.jsonl, one line per stage
    # completion, plus legacy <node>@<visit>.json) are meta context:
    # run-machinery stage records about the workflow itself, consumed later
    # by the improve workflow. They are NO review input —
    # dropped here, so they appear in NO record and NO count.
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

    # Stat-map anchor: the run base. ADR-0032: work-item claims are
    # server-side — no tracker-file claim transition exists to anchor
    # on, so the numstat anchors at the run base directly (named in the
    # header, never silent).
    let diff_desc = ($base.short + " (run base — claims are server-side, no tracker-file claim anchor)")

    # Critical-first emission — the records a tail-truncated render hits
    # last are the least load-bearing (the worktree state).
    let integrity = (integrity-section $base.short $seed_desc $diff_desc $seed_rows $churn_rows $wt.label)
    let stat_map = (stat-map-section $seed_rows $churn_rows)
    let files = (seed-work-files-section $seed_rows)
    let churn = (loop-churn-section $churn_rows)
    # Mixed records (seed work AND loop churn) get an anomaly list
    # (fabro-c0e5); clean or churn-only records skip it.
    let anomaly = (if (not ($seed_rows | is-empty)) and (not ($churn_rows | is-empty)) {
        anomaly-section $churn_rows
    } else { "" })
    let worktree = (worktree-section $wt.lines)

    print ($no_base_note | str trim -r -c "\n")
    print $integrity
    print $stat_map
    print $files
    print $churn
    if ($anomaly | is-not-empty) { print $anomaly }
    print $worktree
}
