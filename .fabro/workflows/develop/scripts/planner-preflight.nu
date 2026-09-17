#!/usr/bin/env nu
# Deterministic already-landed preflight (fabro-a32f): runs BEFORE the
# planner LLM lap and greps the merge-target base history for the top
# `sd ready --assignee fabro` candidates, reusing dup-run-check.nu's
# landed-implementation classification verbatim (filed-only revisor
# commits never count; Fabro-Run trailer identity via --self; a landed
# true-merge or squash `(#n)` commit implementing the seed counts).
#
# Motivation (run 01M2Q2Q2NY211PVV0E44YDAAQN, PR #207): the planner's
# ALREADY LANDED arm re-derived a sub-second greppable fact (manual fix
# commit 34e8f2c vs a stale tracker row) across three LLM probe rounds —
# 93.6s / $0.143 for what this script answers mechanically.
#
# Routes (output_schema="routing" on the node):
#   - TOP candidate carries a foreign landed implementation -> append the
#     self-explaining closure note (sd update, full body + note), then
#     superseded-close (sd close --reason "superseded: fix landed in
#     <sha>"), then emit preferred_next_label "Already landed" — the run
#     exits with no LLM lap at all (same plain exit edge the planner's
#     Already-landing used).
#   - anything else -> preferred_next_label "Preflight done"; the full
#     per-candidate verdict table ships inline to the planner as the
#     `output.preflight` context key, so the LLM adjudicates only the
#     ambiguous residue (goal-named seeds outside the candidate set,
#     borderline classifications) instead of re-grepping base history.
#
# Fail-open (hard rule): ANY internal error — git fetch failure, tracker
# error, empty candidate list, dup-run-check crash, close failure —
# routes the planner normally with the degraded mode recorded in the
# report. This node must never dead-end or block the run; it exits 0 on
# every degraded path and reserves non-zero for invocation bugs.
#
# Relationship to open fabro-a01f (claim race / stale tracker snapshot):
# this preflight NARROWS that window — it catches the merged-but-still-
# open state at pre-planner time — but does NOT close it: a PR can still
# land between this check and the planner's claim, and the tracker
# snapshot can still lag. The durable engine fixes (fabro-6b58,
# fabro-9372) remain the real closures; the implementer-side dup-run
# check stays the backstop.
#
# Usage (node invocation; also drivable standalone for dry-runs):
#   echo <run-id> | nu planner-preflight.nu [--base origin/denkhaus]
#   nu planner-preflight.nu --candidates fabro-x,fabro-y --report-only
#     --candidates  comma-separated override of the sd ready candidate
#                   list (dry-run / fixture path; skips sd ready)
#     --report-only skip the closure side effects — report the landing
#                   verdict without closing anything (dry-run / fixtures)
#     --top N       how many top candidates to check (default 5)

# Bounded per-candidate row for the planner-facing report.
def row [v: record] {
    let m = ($v.implementation_matches? | default [] | first | default {})
    let ce = ($v.closing_evidence? | default {})
    {seed: $v.seed,
     verdict: $v.verdict,
     sha: ($m.sha? | default ($ce.sha? | default null)),
     subject: (if (($m.subject? | default ($ce.subject? | default "")) | is-empty) { null } else { ($m.subject? | default ($ce.subject? | default "")) | str substring 0..120 }),
     filed_only_matches: ($v.filed_only_matches? | default 0)}
}

# Resolve dup-run-check relative to THIS script (.fabro/scripts/ is
# three levels up from .fabro/workflows/develop/scripts/), never CWD —
# the fixture battery drives this script from a scratch clone. `path
# self` is parse-time only, so the anchor must be a const.
const SCRIPT_DIR = (path self | path dirname)

def main [--base: string = "origin/denkhaus", --candidates: string, --report-only, --top: int = 5]: nothing -> nothing {
    # Non-tty stdin (same nu 0.115 constraint as closeout.nu): the engine
    # pipes internal.run_id, read it through external cat.
    let run_id = (cat | str join | str trim)

    mut mode = "checked"
    mut degraded_reason = ""

    # Candidate list: --candidates override, else the top of sd ready.
    let cand = (if $candidates != null {
        $candidates | split row ',' | each {|c| $c | str trim} | where {|c| not ($c | is-empty)}
    } else {
        let r = (do { sd ready --assignee fabro --limit 200 --format json } | complete)
        if $r.exit_code != 0 {
            $mode = "degraded"
            $degraded_reason = $"sd ready failed: ($r.stderr | str trim | str substring 0..200)"
            []
        } else {
            let parsed = (try { $r.stdout | from json } catch { null })
            if $parsed == null {
                $mode = "degraded"
                $degraded_reason = "sd ready output not valid JSON"
                []
            } else {
                # sd ready --format json: {success, command, issues: [...]}
                # (issues is absent/empty when nothing is ready)
                $parsed | get -o issues | default [] | get -o id | default []
            }
        }
    })

    # Empty candidate list is a legitimate degraded park (tracker empty is
    # the PLANNER's verdict to make, never this script's) — never dead-end.
    if ($cand | is-empty) {
        if $mode != "degraded" {
            $mode = "degraded"
            $degraded_reason = "no candidates (empty override or empty sd ready)"
        }
        {"outcome": "succeeded",
         "preferred_next_label": "Preflight done",
         "context_updates": {"output.preflight": {"mode": $mode, "run_id": ($run_id | default null), "candidates": [], "degraded_reason": (if ($degraded_reason | is-empty) { null } else { $degraded_reason }), "closed": null}}} | to json --raw | print
        return
    }

    let ids = ($cand | first ([$top 1] | math max))

    let dup = ($SCRIPT_DIR | path join '../../..' 'scripts' 'dup-run-check.nu')
    let res = (do { nu $dup ...$ids --base $base --self $run_id } | complete)
    if $res.exit_code != 0 {
        {"outcome": "succeeded",
         "preferred_next_label": "Preflight done",
         "context_updates": {"output.preflight": {"mode": "degraded", "run_id": ($run_id | default null), "candidates": [], "degraded_reason": ($"dup-run-check failed: ($res.stderr | str trim | str substring 0..200)"), "closed": null}}} | to json --raw | print
        return
    }
    let verdicts = ($res.stdout | lines | compact | each {|l| $l | from json })

    # Mechanical route: the TOP candidate's verdict is duplicate (for an
    # open sd-ready seed that means a foreign landed implementation).
    let topv = ($verdicts | first | default null)
    let landed = ($topv != null and $topv.verdict == "duplicate")
    let evidence_sha = (if $landed {
        (($topv.implementation_matches? | default [] | first | default {}).sha? | default (($topv.closing_evidence? | default {}).sha? | default null))
    } else { null })

    # Deterministic superseded-close: closure note FIRST (tracker stays
    # self-explaining), then sd close. Any failure aborts the close and
    # falls through to the planner with the evidence inline — never a
    # dead-end, never a close without its note.
    mut closed = {seed: null, sha: null}
    mut close_note = ""
    if $landed and not $report_only {
        let show = (do { sd show $topv.seed --format json } | complete)
        if $show.exit_code != 0 {
            $close_note = $"close aborted: sd show failed — planner adjudicates with evidence inline"
        } else {
            let body = ($show.stdout | from json | get -o issue.description | default "")
            let note = ($body + " + closure note: superseded: fix landed in " + ($evidence_sha | default "?") + " (run " + ($run_id | default "?") + ")")
            let upd = (do { sd update $topv.seed --description $note } | complete)
            if $upd.exit_code != 0 {
                $close_note = $"close aborted: sd update (closure note) failed: ($upd.stderr | str trim | str substring 0..160)"
            } else {
                let cls = (do { sd close $topv.seed --reason $"superseded: fix landed in ($evidence_sha)" } | complete)
                if $cls.exit_code != 0 {
                    $close_note = $"close aborted: sd close failed: ($cls.stderr | str trim | str substring 0..160)"
                } else {
                    $closed = {seed: $topv.seed, sha: $evidence_sha}
                }
            }
        }
    }

    # The Already-landed route requires the close to have happened (or
    # report-only dry-run); otherwise continue so the planner can act on
    # the inline evidence — routing and tracker state never diverge.
    let route = (if $landed and ($report_only or $closed.seed != null) { "Already landed" } else { "Preflight done" })

    let report = {mode: (if (not ($close_note | is-empty)) and $mode == "checked" { "degraded" } else { $mode }),
                  run_id: ($run_id | default null),
                  candidates: ($verdicts | each {|v| row $v}),
                  degraded_reason: (if ($degraded_reason | is-empty) { null } else { $degraded_reason }),
                  closed: $closed}
    {"outcome": "succeeded",
     "preferred_next_label": $route,
     "context_updates": {"output.preflight": $report}} | to json --raw | print
}
