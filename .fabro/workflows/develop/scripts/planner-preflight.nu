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
#   - fabro-ead4: the note+close pair fires for EVERY unambiguous
#     duplicate candidate (verdict exactly "duplicate" WITH a resolvable
#     landed sha), not only the top row — a duplicate with no sha
#     (tracker-closed, unresolvable evidence) is AMBIGUOUS and stays for
#     planner adjudication. Non-top closes are bookkeeping only and never
#     change the route; each close is fail-open per candidate, journaled
#     in the report's close_log. The report also carries a one-line
#     `legend` mapping each verdict string to its meaning so the planner
#     never re-reads this header to interpret the table.
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
# Anchor verification (fabro-7daf): each candidate's description is
# scanned for `path:line` / `path:line-line` anchors, verified against
# the current worktree, and surfaced as anchors_ok/anchor_flags fields on
# the verdict rows. Anchor checks fail open: any parse/read failure
# degrades to "no anchors" (anchors_ok true) exactly like a seed that
# cites no anchors at all.
#
# Usage (node invocation; also drivable standalone for dry-runs):
#   echo <run-id> | nu planner-preflight.nu [--base origin/denkhaus]
#   nu planner-preflight.nu --candidates fabro-x,fabro-y --report-only
#     --candidates  comma-separated override of the sd ready candidate
#                   list (dry-run / fixture path; skips sd ready)
#     --report-only skip the closure side effects — report the landing
#                   verdict without closing anything (dry-run / fixtures)
#     --top N       how many top candidates to check (default 5)

#
# STANDING POLICY (fabro-9ec3, 2026-09-17): any planner rule naming a
# mechanically-checkable invariant lands as a CHECK in this script (or
# the engine's output validator), never as a new prose paragraph in
# prompts/planner.md — the 139-line prompt was the structural cause of
# the largest seed cluster. Migrated arms recorded here:
#   arm 1 (fabro-4c81 intent, fabro-7daf mechanism): pre-claim
#     path-resolution of seed-cited anchors. The existing anchor
#     verification covers path:line/path:line-line anchors against the
#     worktree; fabro-9ec3 extended it to bare path-only citations
#     (anchor_check.nu extract-bare-paths/check-bare-paths) — no gap
#     remains, both flag classes ship in anchor_flags.
#   arm 2 (complements engine-side fabro-9372): in-flight run/PR
#     exclusion — in-flight-claims below maps recent unmerged develop
#     run branches to claimed seed ids via the stage-journal fallback
#     and reports them per-candidate as in_flight/in_flight_run.
#   arm 3: the brief gate-command ban lives in the PLANNER OUTPUT
#     SCHEMA (.fabro/workflows/develop/schemas/planner-output.schema.json,
#     wired via the planner node's output_schema) — the fabro-017f teeth
#     pattern: a brief containing `just qualitygate` or its
#     byte-equivalent body fails validation and burns an output retry,
#     never passes silently.

# Anchor verification helpers (fabro-7daf): cited file:line anchors in
# seed descriptions are checked against the current worktree so rotted
# (dead) seeds surface mechanically in the verdict table below.
source anchor_check.nu

# In-flight run/PR exclusion (fabro-9ec3 arm 2; complements engine-side
# fabro-9372 and the planner prompt's fabro_runs_list guard): a seed id
# claimed by a RECENT, UNMERGED develop run branch is in flight — the
# ~15-min claim-to-PR window plus the open-PR lifetime. Credential-less:
# derives the remote from --base (same as dup-run-check), fetches
# refs/heads/fabro/run/* once into a scratch namespace, keeps branches
# committed within the last 14 days (newest first, max 12), drops
# branches already merged into base (their seed is the landed arm's
# business), skips the invoking run, and maps run -> seed id by grepping
# the run branch's stage journal (.fabro/journal/<run_id>.jsonl — the
# documented PROJECT_FACTS fallback; planner-node lines preferred for
# precision). Fail-open hard rule: any error degrades to zero claims
# with a note, never blocks — a false claim mark is planner-adjudicated,
# this table never closes or skips anything itself.
def journal-claims [sha: string, run: string] {
    let j = (do { ^git show $"($sha):.fabro/journal/($run).jsonl" } | complete)
    if $j.exit_code != 0 { return [] }
    let lines = ($j.stdout | lines | where {|l| $l | str contains '"node":"planner"'})
    if ($lines | is-empty) { return [] }
    $lines
    | parse --regex 'fabro-(?<seed>(?:[a-z][a-z0-9]*-)?[0-9a-z]{4,})(?![0-9a-z-])'
    | get -o seed
    | default []
    | uniq
    | each {|seed| $"fabro-($seed)"}
}

def in-flight-claims [remote: string, base: string, self_id: string] {
    let fetch = (do { ^git fetch -q $remote '+refs/heads/fabro/run/*:refs/preflight-run/fabro/run/*' } | complete)
    if $fetch.exit_code != 0 {
        return {claims: [], note: $"in-flight fetch degraded: ($fetch.stderr | str trim | str substring 0..160)"}
    }
    let refs = (do { ^git for-each-ref '--sort=-committerdate' '--format=%(refname) %(objectname) %(committerdate:unix)' 'refs/preflight-run/' } | complete)
    if $refs.exit_code != 0 {
        return {claims: [], note: "in-flight for-each-ref degraded"}
    }
    let now = (date now | format date "%s" | into int)
    let rows = ($refs.stdout | parse --regex '(?m)^refs/preflight-run/fabro/run/(?P<run>[0-9A-Za-z-]+) (?P<sha>[0-9a-f]{7,40}) (?P<ts>\d+)')
    let recent = ($rows | each {|r| $r | update ts ($r.ts | into int)}
        | where {|r| ($now - $r.ts) < 1209600}
        | first 12)
    mut claims = []
    for r in $recent {
        if $self_id != null and $r.run == $self_id { continue }
        let merged = (do { ^git merge-base --is-ancestor $r.sha $base } | complete)
        if $merged.exit_code == 0 { continue }
        for seed in (journal-claims $r.sha $r.run) {
            $claims = ($claims | append {seed: $seed, run: $r.run})
        }
    }
    let dedup = (if ($claims | is-empty) {
        []
    } else {
        $claims | group-by seed | items {|seed, cs| {seed: $seed, run: ($cs | get run | first)}}
    })
    {claims: $dedup, note: null}
}

# Bounded per-candidate row for the planner-facing report. Anchor fields
# (fabro-7daf): anchors_ok is false when ANY cited file:line anchor in
# the description is missing/rotted/mismatched; anchor_flags carries the
# per-anchor detail. The verdict itself is unchanged — anchor rot routes
# through planner adjudication, never through this script's close path.
def row [v: record, desc: string, root: string, claims: list] {
    let m = ($v.implementation_matches? | default [] | first | default {})
    let ce = ($v.closing_evidence? | default {})
    let flags = ((extract-anchors $desc | each {|a| check-anchor $a $root} | append (check-bare-paths $desc $root)) | where {|f| $f.status != "ok"})
    # in-flight is advisory: never marked for candidates whose
    # implementation already landed (the duplicate close path owns them).
    let hit = (if $v.verdict == "duplicate" { null } else { $claims | where {|c| $c.seed == $v.seed} | first | default null })
    {seed: $v.seed,
     verdict: $v.verdict,
     sha: ($m.sha? | default ($ce.sha? | default null)),
     subject: (if (($m.subject? | default ($ce.subject? | default "")) | is-empty) { null } else { ($m.subject? | default ($ce.subject? | default "")) | str substring 0..120 }),
     filed_only_matches: ($v.filed_only_matches? | default 0),
     anchors_ok: (($flags | length) == 0),
     anchor_flags: $flags,
     in_flight: ($hit != null),
     in_flight_run: (if $hit == null { null } else { $hit.run })}
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
    # Candidates carry their description so cited file:line anchors can
    # be verified (fabro-7daf); a description that cannot be fetched
    # degrades per-candidate to "no anchors", never to a script failure.
    let cand = (if $candidates != null {
        $candidates | split row ',' | each {|c| $c | str trim} | where {|c| not ($c | is-empty)} | each {|id|
            let r = (do { sd show $id --format json } | complete)
            {id: $id, description: (if $r.exit_code != 0 { "" } else { (try { $r.stdout | from json | get -o issue.description | default "" } catch { "" }) })}
        }
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
                $parsed | get -o issues | default [] | each {|i| {id: ($i | get -o id | default ""), description: ($i | get -o description | default "")}} | where {|c| not ($c.id | is-empty)}
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

    let top_c = ($cand | first ([$top 1] | math max))
    let ids = ($top_c | get id)
    let cdesc = ($top_c | get description)

    let dup = ($SCRIPT_DIR | path join '../../..' 'scripts' 'dup-run-check.nu')
    let res = (do { nu $dup ...$ids --base $base --self $run_id } | complete)
    if $res.exit_code != 0 {
        {"outcome": "succeeded",
         "preferred_next_label": "Preflight done",
         "context_updates": {"output.preflight": {"mode": "degraded", "run_id": ($run_id | default null), "candidates": [], "degraded_reason": ($"dup-run-check failed: ($res.stderr | str trim | str substring 0..200)"), "closed": null}}} | to json --raw | print
        return
    }
    let verdicts = ($res.stdout | lines | compact | each {|l| $l | from json })

    # In-flight claims (fabro-9ec3 arm 2): fail-open wrapper — a crash in
    # the arm degrades to zero claims with a note, never blocks the run.
    let remote = ($base | split row '/' | first)
    let inflight = (try { in-flight-claims $remote $base ($run_id | default null) } catch { {claims: [], note: "in-flight arm degraded (fail-open)"} })

    # Mechanical route: the TOP candidate's verdict is duplicate (for an
    # open sd-ready seed that means a foreign landed implementation).
    let topv = ($verdicts | first | default null)
    let landed = ($topv != null and $topv.verdict == "duplicate")

    # Deterministic superseded-close (fabro-ead4): fires for EVERY
    # candidate whose verdict is exactly "duplicate" WITH resolvable
    # implementation evidence (a landed sha) — the unambiguous set. A
    # duplicate WITHOUT a sha (tracker-closed, no closing evidence) is
    # ambiguous and stays open here for planner adjudication. Closure
    # note FIRST (tracker stays self-explaining), then sd close; each
    # candidate is fail-open — one failed note/close is journaled in
    # close_log and never aborts the remaining closures. Non-top closes
    # are bookkeeping only: routing still keys off the TOP candidate.
    # any-typed columns: the record starts null-valued and is later
    # assigned string fields — an inferred record<seed: nothing, ...> would
    # reject that assignment (observed nu 0.115 type_mismatch crash).
    mut closed: record<seed: any, sha: any> = {seed: null, sha: null}
    mut close_log = []
    if not $report_only {
        for v in $verdicts {
            if $v.verdict != "duplicate" { continue }
            let m = ($v.implementation_matches? | default [] | first | default {})
            let sha = ($m.sha? | default (($v.closing_evidence? | default {}).sha? | default null))
            if $sha == null {
                $close_log = ($close_log | append {seed: $v.seed, closed: false, note: "skipped: duplicate without implementation evidence — planner adjudicates"})
                continue
            }
            let show = (do { sd show $v.seed --format json } | complete)
            if $show.exit_code != 0 {
                $close_log = ($close_log | append {seed: $v.seed, closed: false, note: "close aborted: sd show failed — planner adjudicates with evidence inline"})
                continue
            }
            let body_raw = ($show.stdout | from json | get -o issue.description?)
            let body = (if ($body_raw | is-empty) { "" } else { $body_raw })
            let note = ($body + " + closure note: superseded: fix landed in " + $sha + " (run " + ($run_id | default "?") + ")")
            let upd = (do { sd update $v.seed --description $note } | complete)
            if $upd.exit_code != 0 {
                $close_log = ($close_log | append {seed: $v.seed, closed: false, note: ("close aborted: sd update (closure note) failed: " + ($upd.stderr | str trim | str substring 0..160))})
                continue
            }
            let cls = (do { sd close $v.seed --reason $"superseded: fix landed in ($sha)" } | complete)
            if $cls.exit_code != 0 {
                $close_log = ($close_log | append {seed: $v.seed, closed: false, note: ("close aborted: sd close failed: " + ($cls.stderr | str trim | str substring 0..160))})
            } else {
                $close_log = ($close_log | append {seed: $v.seed, closed: true, sha: $sha})
                if $topv != null and $v.seed == $topv.seed {
                    $closed = {seed: $v.seed, sha: $sha}
                }
            }
        }
    }

    # The Already-landed route requires the TOP close to have happened
    # (or report-only dry-run); otherwise continue so the planner can act
    # on the inline evidence — routing and tracker state never diverge.
    let route = (if $landed and ($report_only or $closed.seed != null) { "Already landed" } else { "Preflight done" })

    # Verdict legend (fabro-ead4): one field mapping every verdict string
    # dup-run-check emits to its meaning, so the planner never re-reads
    # the script header to interpret the table.
    let legend = {duplicate: "implementation already in merge-target base (or tracker-closed without resolvable evidence)",
                  clean: "no landed implementation found",
                  degraded: "check failed (fetch/tracker error)"}
    let aborts = ($close_log | where {|e| (not $e.closed) and ($e.note | str starts-with "close aborted")} | length)
    let report = {mode: (if $aborts > 0 and $mode == "checked" { "degraded" } else { $mode }),
                  run_id: ($run_id | default null),
                  candidates: ($verdicts | enumerate | each {|e| row $e.item ($cdesc | get -o $e.index | default "") ($SCRIPT_DIR | path join '../../../..') ($inflight.claims | default [])}),
                  legend: $legend,
                  degraded_reason: (if $aborts > 0 and ($degraded_reason | is-empty) { ("close_log: " + ($aborts | into string) + " superseded-close(s) aborted") } else if ($degraded_reason | is-empty) { null } else { $degraded_reason }),
                  in_flight_note: ($inflight.note? | default null),
                  close_log: $close_log,
                  closed: $closed}
    {"outcome": "succeeded",
     "preferred_next_label": $route,
     "context_updates": {"output.preflight": $report}} | to json --raw | print
}
