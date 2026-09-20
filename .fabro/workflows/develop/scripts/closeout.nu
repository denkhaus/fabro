#!/usr/bin/env nu
# Closeout (ADR-0010): deterministic post-approval bookkeeping.
#
# The reviewer's Approved edge lands HERE. This script closes EXACTLY the
# seed this run worked on — delivered by the engine via stdin
# (stdin_source="current_seed_id" on the node). One seed per run: after a
# successful close the run exits; the next seed is the next run's job
# (user directive 2026-09-04 — bounded runs give the revisor a clear,
# single-seed working field and keep run cost predictable).
#
# The seed id comes from CONTEXT, never from "first in_progress seed":
# the tracker carries parallel claims (stale or live), and run
# 01M1NK4V3YG3AQAMEKDJ6V471F closed two WRONG stale seeds
# that way before this fix.
#
# Never an LLM: run 01M0… measured planner@2 at 21s / $0.021 / ~7% of wall
# time for exactly these mechanical actions. Failure keeps the seed open —
# the next run re-enters the review cycle; an approved-but-unclosed seed is
# re-approvable harmlessly.

# ---------------------------------------------------------------------------
# Dockerfile-touch warning (fabro-6f6e)
#
# WHY: run 01M1YTVK73YEJXW4542MWDX4BJ (seed fabro-05d0) delivered only a
# .fabro/Dockerfile.toolchain edit; Docker is absent from run sandboxes so
# no image build ran, the review approved green — yet the fix stayed INERT
# because fabro-toolchain:noble is BUILT, not mounted, and a Dockerfile*
# edit takes effect only after that image rebuilds. Same class as the gh
# breakage in .fabro/Dockerfile (since 98ed9f1) that still produced exit
# 127 in run containers on Sep 7. This warning names the touched files at
# the moment the close is decided. It must NEVER fail or block the close:
# every git call below is wrapped in `do { ... } | complete` (git errors
# degrade to silence) and the whole check runs under `do -i` — `complete`
# only accepts external commands, so the outer guard is do-i rather than
# do|complete, with the same degrade-to-silence semantics.
#
# SPEC SCOPE: run-summary/PR-body surfacing of this warning is exactly the
# channel open seed fabro-5b0a proposes and does NOT exist yet — this seed
# deliberately does NOT build it. The warning's captured stdout/stderr
# lands in the stage journal and run output, which satisfies this seed;
# fabro-5b0a remains the follow-up for PR-body surfacing.
# ---------------------------------------------------------------------------

# Pure filter over a path list: any path with a SEGMENT matching the glob
# `Dockerfile*` (covers .fabro/Dockerfile.toolchain, .fabro/Dockerfile,
# root Dockerfile, nested x/Dockerfile.dev). Pure over its input so the
# smoke test (closeout-smoke.nu) exercises it without git.
def dockerfile-hits [paths: list<string>]: nothing -> list<string> {
    $paths | where {|p| ($p | split row "/" | any {|seg| $seg | str starts-with "Dockerfile"})}
}

# --- diff-anchor helpers: MIRRORED from evidence.nu (seed-claim-base
# approach) — reuse that anchoring scheme, do not invent a new one.

def current-branch []: nothing -> string {
    git branch --show-current | str trim
}

def run-base []: nothing -> record<base: string, short: string, grounded: bool> {
    let run_id = (
        current-branch
        | parse --regex 'fabro/run/(?P<id>[^/]+)$'
        | get -o id.0
        | default ''
    )
    let subject_mark = $"fabro\(($run_id)\):"
    let checkpoints = (do { git log --format=%H --fixed-strings --grep $subject_mark } | complete | get stdout | lines | compact)
    if ($checkpoints | is-empty) {
        {base: "HEAD", short: (do { git rev-parse --short HEAD } | complete | get stdout | str trim), grounded: false}
    } else {
        let base = (do { git rev-parse $"($checkpoints | last)^" } | complete | get stdout | str trim)
        {base: $base, short: (do { git rev-parse --short $base } | complete | get stdout | str trim), grounded: true}
    }
}

def seed-status-at [commit: string, seed_id: string]: nothing -> any {
    let res = (do { git show $"($commit):.seeds/issues.jsonl" } | complete)
    if $res.exit_code != 0 { return null }
    let rows = ($res.stdout | lines | compact | each {|l| do -i { $l | from json } })
    let hit = ($rows | where {|r| $r != null and ($r | get -o id | default '') == $seed_id } | get -o 0 | default null)
    if $hit == null { null } else { $hit | get -o status | default null }
}

# Newest commit where the seed TRANSITIONS to in_progress (in_progress at
# C, not at C^); the claim commit itself is the base. Fallback: run base.
# See evidence.nu seed-claim-base for the full rationale.
def seed-claim-base [seed_id: string, run_base: record]: nothing -> record<base: string, short: string, grounded: bool, fallback: bool> {
    for c in (do { git log --format=%H -- .seeds/issues.jsonl } | complete | get stdout | lines | compact) {
        if ((seed-status-at $c $seed_id) == "in_progress") {
            let parent = (do { git rev-parse $"($c)^" } | complete)
            if $parent.exit_code == 0 {
                let before = (seed-status-at ($parent.stdout | str trim) $seed_id)
                if $before != "in_progress" {
                    return {base: $c, short: (do { git rev-parse --short $c } | complete | get stdout | str trim), grounded: true, fallback: false}
                }
            }
        }
    }
    {base: $run_base.base, short: $run_base.short, grounded: $run_base.grounded, fallback: true}
}

# Best-effort detection + warning: warn when the seed's diff (claim-base
# anchored, fallback run base — flagged in the base itself) touches any
# Dockerfile* path. Prints the warning to BOTH stderr and stdout (both are
# captured into the stage journal / run output); never blocks the close.
def warn-dockerfile-diff [seed_id: string]: nothing -> nothing {
    let base = (seed-claim-base $seed_id (run-base))
    let res = (do { git diff --name-only $base.base } | complete)
    if $res.exit_code != 0 { return }
    let hits = (dockerfile-hits ($res.stdout | lines | compact))
    if ($hits | is-empty) { return }
    let msg = $"closeout: WARNING — diff touches Dockerfile path\(s\): ($hits | str join ', '). This fix is INERT until the fabro-toolchain:noble image is rebuilt — run sandboxes build that image and never mount it live."
    print -e $msg
    print $msg
}

# ---------------------------------------------------------------------------
# Closure-discipline pre-close check (fabro-02c4)
#
# WHY: seed fabro-9967 was closed 2026-09-09 15:42 (in a seeds:sync)
# although its demand (sd ready cap + created_since in planner.md) was
# never implemented — NO implementing diff existed — and the revisor
# caught it only by accident while deduping (run 01M23J61HH8Z,
# regression finding). Before its `sd close`, this gate verifies the
# run actually delivered a diff the seed's demand is visible in
# (claim-base anchored — the same seed-claim-base/run-base helpers the
# Dockerfile warning uses). If not visible, the run does NOT close: the
# seed stays OPEN and a PARK note goes to stdout AND stderr (both land
# in the stage journal / run output) for the next cycle to act on.
#
# Tracker/journal bookkeeping (`.seeds`, `.fabro/journal`) is excluded
# from the patch: those files mechanically echo seed metadata (the
# claim itself, journal records quoting the brief) and would satisfy
# the match with zero implementation.
#
# Degrade-to-close: any git/sd failure inside the verdict returns
# visible=true — this gate must never block a legitimate close on
# tooling error (same philosophy as the Dockerfile warning above). An
# empty token set (title unparsable) degrades to a non-empty-patch
# check.
# ---------------------------------------------------------------------------

# Pure: distinctive demand tokens from a seed title — lowercase
# alphanumeric runs of length >= 4, minus function-word stopwords.
def demand-tokens [title: string]: nothing -> list<string> {
    let stopwords = [seed task fabro with from that this when then they them will must into else only than have been does were what which where while about]
    $title | str lowercase | split row --regex '[^a-z0-9]+' | where {|t|
        ($t | str length) >= 4 and not ($t in $stopwords)
    } | uniq
}

# Pure: is the seed's demand literally visible in the run patch? Empty
# patch -> never visible (the fabro-9967 shape: closed with no
# implementing diff). Empty tokens -> any non-empty patch passes
# (degrade). Otherwise ANY distinctive title token found in the patch
# counts: the patch is this run's own claim-anchored work, so a hit
# means the run touched what the seed names.
def demand-visible [tokens: list<string>, patch: string]: nothing -> bool {
    if ($patch | str trim | is-empty) { return false }
    if ($tokens | is-empty) { return true }
    $tokens | any {|t| $patch | str contains $t }
}

# Best-effort visibility verdict for THIS seed against the run diff
# (claim-base anchored, fallback run base). Never blocks on git/sd
# failure — degrades to visible.
def seed-demand-visible [seed_id: string]: nothing -> bool {
    let base = (seed-claim-base $seed_id (run-base))
    let diff_res = (do { git diff $base.base -- . ':(exclude).seeds' ':(exclude).fabro/journal' } | complete)
    if $diff_res.exit_code != 0 { return true }
    let title = (do -i { sd show $seed_id --format json | from json | get issue.title } | default '')
    let tokens = (do -i { demand-tokens $title } | default [])
    demand-visible $tokens $diff_res.stdout
}

# ---------------------------------------------------------------------------
# Reviewer-journal non-blocking sweep (fabro-22fa)
#
# WHY: a reviewer that approves with residual defects notes them in its
# journal observations with explicit non-blocking wording ("non-blocking",
# "noted but not blocking", "not blocking"). Before fabro-22fa those
# findings lived only in `.fabro/journal/<run_id>.jsonl` — when closeout
# ran `sd close`, the finding died with the closed seed: no open brief
# existed to fold the follow-up into. This sweep re-files each explicitly
# non-blocking reviewer observation as a NEW open seed BEFORE the close,
# with provenance (finding text, closed seed id, source run id) in the
# body so the finding survives closure traceably.
#
# ADVISORY ONLY: every failure (journal missing, JSON unparsable, sd
# create error) logs to stderr and NEVER blocks the close — same
# degrade-to-silence philosophy as warn-dockerfile-diff (`do -i` +
# `do { ... } | complete` wrappers). Null path: a journal with no
# non-blocking reviewer findings yields an empty list and ZERO sd create
# calls — byte-identical close semantics.
# ---------------------------------------------------------------------------

# Pure: does a reviewer observation carry an explicit non-blocking
# marker? Case-insensitive; "not blocking" also covers the "noted but
# not blocking" phrasing. An observation merely mentioning a blocking
# defect ("this is blocking") does NOT match.
def is-nonblocking [obs: string]: nothing -> bool {
    let low = ($obs | str lowercase)
    ($low | str contains "non-blocking") or ($low | str contains "not blocking")
}

# Pure: non-blocking findings from journal JSONL text — reviewer-node
# records only, their observations, filtered by is-nonblocking.
def nonblocking-from-journal [text: string]: nothing -> list<string> {
    let obs = (
        $text | lines | compact
        | each {|l| do -i { $l | from json } }
        | where {|r| ($r | describe | str starts-with "record") and (($r | get -o node | default '') == "reviewer")}
        | get -o data
        | where {|d| $d != null}
        | each {|d| $d | get -o observations | default []}
        | flatten
    )
    $obs | where {|o| is-nonblocking $o}
}

# Best-effort file-level read: missing/unreadable journal -> empty list.
def journal-nonblocking [journal_path: string]: nothing -> list<string> {
    do -i { nonblocking-from-journal (open --raw $journal_path) } | default []
}

# Pure: run id from the run branch (`fabro/run/<run_id>`), '' off-run.
def current-run-id []: nothing -> string {
    current-branch | parse --regex 'fabro/run/(?P<id>[^/]+)$' | get -o id.0 | default ''
}

# Pure: filed-seed title — short finding excerpt plus provenance.
def finding-title [finding: string, seed_id: string]: nothing -> string {
    let excerpt = (if ($finding | str length) > 70 { $finding | str substring 0..69 } else { $finding })
    $"Reviewer residual \(non-blocking\) from ($seed_id): ($excerpt)"
}

# Pure: labels for a residual seed filed by the advisory sweep. Marks
# machine-filed provenance so the planner pool can tell a residual from
# user-assigned work (fabro-2ab8); `sd create` takes comma-labels.
def residual-seed-labels []: nothing -> list<string> {
    ["residual"]
}

# Advisory sweep: file each non-blocking reviewer finding as an open
# seed (type bug, assignee fabro so the develop line can pick it up,
# labels `residual` so the planner sees the machine-filed provenance).
# Never raises: caller wraps in `do -i`; internal sd failures print to
# stderr and continue.
def sweep-reviewer-findings [seed_id: string, run_id: string, journal_path: string]: nothing -> nothing {
    for finding in (journal-nonblocking $journal_path) {
        let desc = $"Residual defect the reviewer explicitly flagged as non-blocking while approving ($seed_id).\n\nFinding text: \"($finding)\"\n\nOrigin: closed seed ($seed_id), reviewer journal ($journal_path).\nBasis: run ($run_id), closed seed ($seed_id)"
        let res = (do { sd create --title (finding-title $finding $seed_id) --description $desc --type bug --assignee fabro --labels (residual-seed-labels | str join ",") } | complete)
        if $res.exit_code != 0 {
            print -e $"closeout: WARNING — could not file reviewer finding as a seed \(non-blocking, from ($seed_id)\): ($res.stderr | str trim)"
        } else {
            print $"closeout: filed reviewer finding \(non-blocking\) as open seed: ($res.stdout | str trim)"
        }
    }
}

def main []: nothing -> nothing {
    # Non-tty stdin: nu 0.115's `input` only works on a tty and raises an
    # I/O error on pipes (run 01M1PVMS7B6N39MG0041C5F7P6) — the engine pipes
    # the context value, so read it through an external `cat`, which inherits
    # the piped stdin.
    let raw = (cat | str join)
    let seed_id = ($raw | str trim)
    if ($seed_id | is-empty) {
        print -e "closeout: stdin carried no seed id (stdin_source misconfigured?)"
        exit 1
    }

    # Dockerfile-touch warning (fabro-6f6e): advisory only — `do -i` plus
    # the per-call complete wrappers above guarantee no failure here can
    # reach the close below. No Dockerfile touched: byte-identical close
    # semantics (stdin validation, sd close, exit codes).
    do -i { warn-dockerfile-diff $seed_id } | ignore

    # Closure-discipline gate (fabro-02c4): PARK instead of closing when
    # the run diff shows no implementing change for the seed's demand.
    # Seed stays open; exit 0 — a park is a deliberate hold for the next
    # cycle, not a stage failure.
    if not (do -i { seed-demand-visible $seed_id } | default true) {
        let park = $"closeout: PARK — \($seed_id\) NOT closed: the run diff \(claim-base anchored, tracker/journal bookkeeping excluded\) shows no implementing change for the seed's demand. fabro-9967 class: closed 2026-09-09 with no implementing diff, caught in run 01M23J61HH8Z. Seed left OPEN — re-enter the review cycle or route Blocked."
        print -e $park
        print $park
        exit 0
    }

    # Reviewer-journal sweep (fabro-22fa): re-file explicitly non-blocking
    # reviewer findings as open seeds BEFORE the close, labeled `residual`
    # (fabro-2ab8) so their provenance is machine-visible in the planner
    # pool. Advisory only —
    # `do -i` plus the complete-wrapped sd calls inside guarantee no
    # failure here can reach the close below. After the PARK gate so a
    # parked (still-open) seed does not double-file on its re-run's
    # close. Null path: no findings -> zero sd create calls.
    let run_id = (do -i { current-run-id } | default '')
    do -i { sweep-reviewer-findings $seed_id $run_id $".fabro/journal/($run_id).jsonl" } | ignore

    let res = (do { sd close $seed_id } | complete)
    if $res.exit_code != 0 {
        print -e $"closeout: sd close ($seed_id) failed: ($res.stderr | str trim)"
        exit 1
    }
    print $"closeout: closed ($seed_id) — one seed per run, exiting"
}
