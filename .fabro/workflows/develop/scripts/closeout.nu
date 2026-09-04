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
# the merged-world tracker carries parallel platform claims (stale or
# live), and run 01M1NK4V3YG3AQAMEKDJ6V471F closed two WRONG stale seeds
# that way before this fix.
#
# Never an LLM: run 01M0… measured planner@2 at 21s / $0.021 / ~7% of wall
# time for exactly these mechanical actions. Failure keeps the seed open —
# the next run re-enters the review cycle; an approved-but-unclosed seed is
# re-approvable harmlessly.

def main []: nothing -> nothing {
    let raw = (input)
    let seed_id = ($raw | str trim)
    if ($seed_id | is-empty) {
        print -e "closeout: stdin carried no seed id (stdin_source misconfigured?)"
        exit 1
    }

    let res = (do { sd close $seed_id } | complete)
    if $res.exit_code != 0 {
        print -e $"closeout: sd close ($seed_id) failed: ($res.stderr | str trim)"
        exit 1
    }
    print $"closeout: closed ($seed_id) — one seed per run, exiting"
}
