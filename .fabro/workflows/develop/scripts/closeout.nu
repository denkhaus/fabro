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

    let res = (do { sd close $seed_id } | complete)
    if $res.exit_code != 0 {
        print -e $"closeout: sd close ($seed_id) failed: ($res.stderr | str trim)"
        exit 1
    }
    print $"closeout: closed ($seed_id) — one seed per run, exiting"
}
