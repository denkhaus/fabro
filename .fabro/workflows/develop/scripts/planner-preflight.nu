#!/usr/bin/env nu
# Deterministic already-landed preflight (fabro-a32f): runs BEFORE the
# planner LLM lap and greps the merge-target base history for the top
# `seeds ready --assignee fabro` candidates, reusing dup-run-check.nu's
# landed-implementation classification verbatim (filed-only revisor
# commits never count; Fabro-Run trailer identity via --self; a landed
# true-merge or squash `(#n)` commit implementing the seed counts).
#
# Motivation (run 01M2Q2Q2NY211PVV0E44YDAAQN, PR #207): the planner's
# ALREADY LANDED arm re-derived a sub-second greppable fact (manual fix
# commit 34e8f2c vs a stale tracker row) across three LLM probe rounds —
# 93.6s / $0.143 for what this script answers mechanically.
#
# Routes (output_schema="routing" on the node) — REPORT-ONLY
# (fabro-83df, user decision 2026-09-19): this script closes NOTHING and
# never exits the run early. It always emits preferred_next_label
# "Preflight done"; the full per-candidate verdict table ships inline to
# the planner as the `output.preflight` context key. The planner's
# ALREADY LANDED arm (fabro-d183 two-branch rule) owns the decision AND
# any superseded-close — every closure stays review-protected. Rationale
# (incident 2026-09-19): a commit subject naming a seed id is not
# evidence that the seed's acceptance criteria are met; the auto-close
# fired once in 34 runs, on reopen/verify commits (fabro-395b), and the
# early exit crashed the line on the planner goal gate (fabro-92e2).
# The report carries a one-line `legend` mapping each verdict string to
# its meaning so the planner never re-reads this header to interpret the
# table; verdicts are ADVISORY — a `duplicate` row is a strong hint, the
# acceptance-criteria judgment stays with the planner.
#
# Fail-open (hard rule): ANY internal error — git fetch failure, tracker
# error, empty candidate list, dup-run-check crash — routes the planner
# normally with the degraded mode recorded in the report. This node must
# never dead-end or block the run; it exits 0 on every degraded path and
# reserves non-zero for invocation bugs.
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
#     --candidates  comma-separated override of the seeds ready candidate
#                   list (dry-run / fixture path; skips seeds ready)
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
#     (anchor_check.nu extract-bare-paths/check-bare-paths); the same
#     shared seam applies the creation-intent window (anchor_check.nu
#     drop-proposed, fabro-f2eb): a citation naming a file the seed
#     itself proposes to create is NOT a missing_file flag. Both real
#     flag classes ship in anchor_flags. The TEETH for
#     fabro-4c81 live in the claim gate (claim-check.nu), not here:
#     this table stays report-only (fabro-83df), the claim node blocks.
#   arm 2 (complements engine-side fabro-9372): in-flight run/PR
#     exclusion — in-flight-claims below maps recent unmerged develop
#     run branches to claimed seed ids via the stage-journal fallback
#     and reports them per-candidate as in_flight/in_flight_run.
#     fabro-32db (2026-09-19): a claim is in flight ONLY while its run
#     is NON-TERMINAL — provably-terminal branches (tip subject
#     `closeout`, squash `(#n)`, or `(failed)` older than the retry
#     grace window; terminal-tip? below) are excluded, so an orphaned
#     terminal-run claim never blocks candidate selection.
#   RETIRED with fabro-83df (2026-09-19): the former live-close arm
#     (fabro-ead4 note+close for every unambiguous duplicate) and the
#     report-only/live route split — report-only is the only mode now.
#   arm 4 (fabro-a8d5, complements the planner prompt's salvage-pointer
#     rule): stranded-implementation scan — a candidate description
#     citing a stranded-branch SALVAGE source is verified mechanically
#     per pointer: canonical bullet parsed, commit resolvable, commit
#     NOT already in base, salvage-source branch still fetchable; a
#     salvage-source run branch cited WITHOUT the canonical pointer is
#     flagged pointer_missing. Fail-open advisory: every flag routes
#     through planner adjudication (see stranded-flags below).
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

# Shared seed-id predicate: journal-claims below mints seed ids from run
# journal text and must accept BOTH shapes (short + family) without
# minting ids from ordinary repo words riding the fabro- prefix.
use ../../../scripts/lib/seed_id.nu seed-ids-in

# Run terminality, tip-subject form (fabro-32db): the shared definition
# is "a seed claim is in flight ONLY while its run is non-terminal".
# Git-only terminality proof is the run branch's TIP COMMIT SUBJECT —
# every stage completion lands as `fabro(<run>): <node> (<status>)`, so:
#   - tip node `closeout` (the develop graph's terminal node, any
#     status) -> the run completed its graph: TERMINAL;
#   - tip subject ending `(#n)` -> the PR squash-merged: TERMINAL;
#   - tip status `failed` -> terminal ONLY past a retry-grace window:
#     the engine retries/bounces failed stages within minutes (observed
#     `evidence (failed)` then `tester (succeeded)` in run
#     01M2RJSBP8XSPCKTEACR7E1PYP), so a FRESH failed tip may still move;
#   - anything else (intermediate `(succeeded)` tip, unparseable
#     subject, git log failure) -> NOT provably terminal, stays
#     in-flight — fail-open direction preserved (advisory, never
#     blocks). NOTE: the journal-last-record form of the hypothesis is
#     FALSE for failed runs (a failed run's journal ends mid-flight
#     with no closeout record — run 01M2WAY0KH's journal stops at
#     `evidence`); the tip subject carries the status the journal drops.
const TERMINAL_GRACE_MIN_DEFAULT = 60

def terminal-tip? [run: string, subject: string, age_sec: int, grace_min: float]: nothing -> bool {
    let s = ($subject | str trim)
    # starts/ends-with over interpolated regexes: nu's $'...' treats
    # every bare (...) as interpolation, so paren-heavy regex cannot be
    # interpolated safely — prefix/suffix matching needs no escapes.
    if ($s | str starts-with $"fabro\(($run)\): closeout ") { return true }
    if ($s =~ '\(#\d+\)$') { return true }
    if ($s | str starts-with $"fabro\(($run)\): ") and ($s | str ends-with " (failed)") {
        return ($age_sec > ($grace_min * 60))
    }
    false
}

# In-flight run/PR exclusion (fabro-9ec3 arm 2; complements engine-side
# fabro-9372 and the planner prompt's fabro_runs_list guard): a seed id
# claimed by a RECENT, UNMERGED, NON-TERMINAL develop run branch is in
# flight — the ~15-min claim-to-PR window plus the open-PR lifetime.
# Terminal-run branches (fabro-32db) are excluded: their claims are
# orphaned, not in flight — a requeued seed must not stay marked
# in-flight forever by the dead run's unmerged branch (the af22
# deadlock: guard requeues at 6h, preflight re-marks, planner re-skips). Credential-less:
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
    # Shared predicate (lib/seed_id.nu): the old inline wide accept-set
    # minted ids from ordinary repo words ('fabro-workflow' -> 'workflow').
    seed-ids-in ($lines | str join "\n")
}

def in-flight-claims [remote: string, base: string, self_id: string] {
    let fetch = (do { ^git fetch -q $remote '+refs/heads/fabro/run/*:refs/preflight-run/fabro/run/*' } | complete)
    if $fetch.exit_code != 0 {
        return {claims: [], note: $"in-flight fetch degraded: ($fetch.stderr | str trim | str substring 0..160)", fetched: false}
    }
    let refs = (do { ^git for-each-ref '--sort=-committerdate' '--format=%(refname) %(objectname) %(committerdate:unix)' 'refs/preflight-run/' } | complete)
    if $refs.exit_code != 0 {
        return {claims: [], note: "in-flight for-each-ref degraded", fetched: false}
    }
    let now = (date now | format date "%s" | into int)
    let rows = ($refs.stdout | parse --regex '(?m)^refs/preflight-run/fabro/run/(?P<run>[0-9A-Za-z-]+) (?P<sha>[0-9a-f]{7,40}) (?P<ts>\d+)')
    let recent = ($rows | each {|r| $r | update ts ($r.ts | into int)}
        | where {|r| ($now - $r.ts) < 1209600}
        | first 12)
    let grace_min = ($env.FABRO_PREFLIGHT_TERMINAL_GRACE_MIN? | default $TERMINAL_GRACE_MIN_DEFAULT | into float)
    mut claims = []
    for r in $recent {
        if $self_id != null and $r.run == $self_id { continue }
        let merged = (do { ^git merge-base --is-ancestor $r.sha $base } | complete)
        if $merged.exit_code == 0 { continue }
        # Terminal-run exclusion (fabro-32db): a provably-terminal
        # branch's claims are orphaned, not in flight. Fail-open: a git
        # log failure leaves the branch treated as in-flight (advisory).
        let tip = (do { ^git log -1 --format=%s $r.sha } | complete)
        if $tip.exit_code == 0 and (terminal-tip? $r.run $tip.stdout ($now - $r.ts) $grace_min) { continue }
        for seed in (journal-claims $r.sha $r.run) {
            $claims = ($claims | append {seed: $seed, run: $r.run})
        }
    }
    let dedup = (if ($claims | is-empty) {
        []
    } else {
        $claims | group-by seed | items {|seed, cs| {seed: $seed, run: ($cs | get run | first)}}
    })
    # `fetched` tells the stranded-implementation arm (below) whether
    # branch-presence checks are knowable at all this run: the
    # refs/preflight-run/ namespace exists ONLY when this fetch and the
    # for-each-ref both succeeded. Fail-open consumers default it false.
    {claims: $dedup, note: null, fetched: true}
}

# Stranded-implementation scan (fabro-a8d5; standing-policy arm 4).
# Complements the planner prompt's salvage-pointer rule: a seed whose
# prior work sits on a stranded run branch must cite the IMPLEMENTER
# COMMIT via the canonical bullet
#   `salvage: git show <commit> on branch <branch>`
# — this arm verifies every such pointer MECHANICALLY at claim time so
# the planner never re-derives it:
#   commit_missing   the pointer names a commit that no longer resolves
#                    (rotted short id, rewritten branch);
#   already_in_base  the salvage already landed on the merge target —
#                    the planner's ALREADY LANDED two-branch-rule arm
#                    owns that seed, not the salvage arm;
#   branch_missing   the salvage-source run branch is no longer fetched
#                    (deleted remote branch) — knowable ONLY when the
#                    in-flight fetch succeeded (refs_fetched), else no
#                    flag (fail-open);
#   pointer_missing  a salvage-source run branch is cited on a
#                    salvage-mentioning line WITHOUT the canonical
#                    commit bullet — the pointer-format rule is
#                    violated; the planner resolves the commit BEFORE
#                    shipping the brief.
# ADVISORY only (the fabro-83df contract): flags never close, skip, or
# route — they are planner input. Fail-open: the whole arm is wrapped
# in try/catch at the call site; git failures degrade to fewer flags,
# never to a blocked run. Precedence per pointer: commit_missing, then
# already_in_base, then branch_missing — the first true problem wins,
# so a row's flags name at most one problem per pointer.
def salvage-pointers [desc: string]: nothing -> list {
    if not ($desc | str lowercase | str contains "salvage") { return [] }
    $desc | parse --regex '(?i)salvage:\s*git show (?P<commit>[0-9a-f]{7,40}) on branch (?P<branch>\S+)'
}

def stranded-flags [desc: string, base: string, refs_fetched: bool]: nothing -> list {
    let pointers = (salvage-pointers $desc)
    mut flags = []
    for p in $pointers {
        # Single-quoted: the peel `^{commit}` must reach git verbatim —
        # nu's $'...' string treats bare braces as interpolation risk.
        let spec = ($p.commit + '^{commit}')
        let found = (do { ^git rev-parse --verify --quiet $spec } | complete)
        if $found.exit_code != 0 {
            $flags = ($flags | append {kind: "salvage_pointer", commit: $p.commit, branch: $p.branch, problem: "commit_missing"})
            continue
        }
        let in_base = (do { ^git merge-base --is-ancestor $p.commit $base } | complete)
        if $in_base.exit_code == 0 {
            $flags = ($flags | append {kind: "salvage_pointer", commit: $p.commit, branch: $p.branch, problem: "already_in_base"})
            continue
        }
        if $refs_fetched and ($p.branch | str starts-with "fabro/run/") {
            let present = (do { ^git show-ref --verify --quiet ("refs/preflight-run/" + $p.branch) } | complete)
            if $present.exit_code != 0 {
                $flags = ($flags | append {kind: "salvage_pointer", commit: $p.commit, branch: $p.branch, problem: "branch_missing"})
            }
        }
    }
    # Pointer-format violations: a salvage-mentioning line that names a
    # run branch but carries no canonical commit pointer (the planner
    # salvage-pointer rule requires the resolved IMPLEMENTER COMMIT
    # before the brief ships).
    if ($pointers | is-empty) {
        for b in ($desc | parse --regex '(?i)salvage[^\n]*?fabro/run/(?P<run>[0-9A-Za-z-]+)') {
            $flags = ($flags | append {kind: "salvage_pointer", commit: null, branch: ("fabro/run/" + $b.run), problem: "pointer_missing"})
        }
    }
    $flags
}

# Bounded per-candidate row for the planner-facing report. Anchor fields
# (fabro-7daf): anchors_ok is false when ANY cited file:line anchor in
# the description is missing/rotted/mismatched; anchor_flags carries the
# per-anchor detail. The verdict itself is unchanged — anchor rot routes
# through planner adjudication, never through this script's close path.
# Envelope-risk moved to the shared seam (fabro-bd55): WHICH surface is
# at risk is lane-relative — anchor_check.nu `lane-surface-risk?` holds
# both token sets (product lane: loop-asset/external surfaces the
# develop envelope hides, the fabro-c4be class kills 3351 x2, f22b,
# 1f52; loop lane: PRODUCT surfaces — the misroute detector, born from
# the fabro-b659 false positive in run 01M58AHT). Fixtures pin both.
# ADVISORY in both directions: a body mention is not proof (the foreign
# path may be cited as BASIS while the primary fix surface is in-lane) —
# the planner verifies the primary surface before claiming.

def row [v: record, desc: string, root: string, claims: list, base: string, refs_fetched: bool, lane: string] {
    let m = ($v.implementation_matches? | default [] | first | default {})
    let ce = ($v.closing_evidence? | default {})
    let checked = ((extract-anchors $desc | each {|a| check-anchor $a $root}) | append (check-bare-paths $desc $root))
    let flags = ((drop-proposed $desc $checked) | where {|f| $f.status != "ok"})
    # fabro-d95c: the Basis-line commit sha and its resolvability in this
    # object DB (null = no sha in body, or tooling fail-open). Advisory —
    # the legend spells the adjudication: never a stale-basis verdict.
    let bc = (extract-basis-commit $desc)
    # Stranded-implementation scan (arm 4): try/catch is the fail-open
    # hard rule — an arm crash degrades to zero flags, never blocks.
    let sflags = (try { stranded-flags $desc $base $refs_fetched } catch { [] })
    # in-flight is advisory: never marked for candidates whose
    # implementation already landed (the duplicate close path owns them).
    let hit = (if $v.verdict == "duplicate" { null } else { $claims | where {|c| $c.seed == $v.seed} | first | default null })
    {seed: $v.seed,
     verdict: $v.verdict,
     sha: ($m.sha? | default ($ce.sha? | default null)),
     subject: (if (($m.subject? | default ($ce.subject? | default "")) | is-empty) { null } else { ($m.subject? | default ($ce.subject? | default "")) | str substring 0..120 }),
     # fabro-4401: surface the closure class of the top match (impl first,
     # else closing evidence) so a foreign closure reads as the advisory
     # input it is — never a mechanical close instruction.
     closure: ($m.closure? | default ($ce.closure? | default null)),
     closure_source: (if ($m.sha? | default null) != null { "implementation" } else if ($ce.sha? | default null) != null { "closing_evidence" } else { null }),
     filed_only_matches: ($v.filed_only_matches? | default 0),
     mention_only_matches: ($v.mention_only_matches? | default 0),
     anchors_ok: (($flags | length) == 0),
     anchor_flags: $flags,
     basis_commit: $bc,
     basis_commit_resolves: (if $bc == null { null } else { basis-commit-resolves $bc $root }),
     in_flight: ($hit != null),
     in_flight_run: (if $hit == null { null } else { $hit.run}),
     salvage_ok: (($sflags | length) == 0),
     salvage_flags: $sflags,
     envelope_risk: (lane-surface-risk? $desc $lane)}
}

# Resolve dup-run-check relative to THIS script (.fabro/scripts/ is
# three levels up from .fabro/workflows/develop/scripts/), never CWD —
# the fixture battery drives this script from a scratch clone. `path
# self` is parse-time only, so the anchor must be a const.
const SCRIPT_DIR = (path self | path dirname)


# Lane parameter (fabro-70b5): the preflight serves BOTH lanes — develop
# (default, assignee fabro) and the loop meta lane (--assignee loop,
# candidates from the loop queue). The envelope_risk flag is LANE-SCOPED
# (fabro-bd55): the shared seam computes it against the lane's own fs
# envelope — product lanes flag loop-asset/external surfaces (skip unless
# the primary surface is repo-visible); the loop lane flags PRODUCT
# surfaces (the misroute detector the loop planner's surface router
# adjudicates).
def main [--base: string = "origin/denkhaus", --candidates: string, --top: int = 5, --assignee: string = "fabro"]: nothing -> nothing {
    # Petri has no internal.run_id run state (fabro-96c6): the invoking
    # run id comes from FABRO_RUN_ID when something binds it; empty
    # degrades the self-closure identity to a no-op exclusion.
    let run_id = ($env.FABRO_RUN_ID? | default "" | str trim)

    mut mode = "checked"
    mut degraded_reason = ""

    # Candidate list: --candidates override, else the top of seeds ready.
    # Candidates carry their description so cited file:line anchors can
    # be verified (fabro-7daf); a description that cannot be fetched
    # degrades per-candidate to "no anchors", never to a script failure.
    let cand = (if $candidates != null {
        $candidates | split row ',' | each {|c| $c | str trim} | where {|c| not ($c | is-empty)} | each {|id|
            let r = (do { seeds show $id --format json } | complete)
            {id: $id, description: (if $r.exit_code != 0 { "" } else { (try { $r.stdout | from json | get -o issue.description | default "" } catch { "" }) })}
        }
    } else {
        let r = (do { seeds ready --assignee $assignee --limit 200 --format json } | complete)
        if $r.exit_code != 0 {
            $mode = "degraded"
            $degraded_reason = $"seeds ready failed: ($r.stderr | str trim | str substring 0..200)"
            []
        } else {
            let parsed = (try { $r.stdout | from json } catch { null })
            if $parsed == null {
                $mode = "degraded"
                $degraded_reason = "seeds ready output not valid JSON"
                []
            } else {
                # seeds ready --format json: {success, command, issues: [...]}
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
            $degraded_reason = "no candidates (empty override or empty seeds ready)"
        }
        {"outcome": "succeeded",
         "preferred_next_label": "Preflight done",
         "context_updates": {"output.preflight": {"mode": $mode, "run_id": ($run_id | default null), "candidates": [], "degraded_reason": (if ($degraded_reason | is-empty) { null } else { $degraded_reason })}}} | to json --raw | print
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
         "context_updates": {"output.preflight": {"mode": "degraded", "run_id": ($run_id | default null), "candidates": [], "degraded_reason": ($"dup-run-check failed: ($res.stderr | str trim | str substring 0..200)")}}} | to json --raw | print
        return
    }
    let verdicts = ($res.stdout | lines | compact | each {|l| $l | from json })

    # In-flight claims (fabro-9ec3 arm 2): fail-open wrapper — a crash in
    # the arm degrades to zero claims with a note, never blocks the run.
    let remote = ($base | split row '/' | first)
    let inflight = (try { in-flight-claims $remote $base ($run_id | default null) } catch { {claims: [], note: "in-flight arm degraded (fail-open)", fetched: false} })

    # Report-only (fabro-83df, 2026-09-19): no closures, no early exit.
    # The former mechanical superseded-close block (fabro-ead4) is
    # retired — the planner's ALREADY LANDED arm owns decisions and
    # closures; verdicts below are advisory input to that arm.
    # Verdict legend (fabro-ead4): one field mapping every verdict string
    # dup-run-check emits to its meaning, so the planner never re-reads
    # the script header to interpret the table.
    let legend = {duplicate: "advisory: implementation possibly already in merge-target base (or tracker-closed without resolvable evidence) — planner judges acceptance criteria and closes per the two-branch rule",
                  clean: "no landed implementation found",
                  degraded: "check failed (fetch/tracker error)"}
    # fabro-bd55: the envelope legend is LANE-SCOPED like the flag —
    # loop readers get the misroute-detector semantics, product lanes the
    # historic hidden-surface advisory.
    let envelope_legend = (if $assignee == "loop" {
        {envelope_risk: "advisory (loop lane): the body names a PRODUCT surface (lib/ apps/ docs/ public/) outside the loop implementer's fs envelope — verify the PRIMARY fix surface: product primary = MISROUTED to @fabro (skip, journal per the surface router); loop-asset primary = claimable (the product path is a BASIS citation)"}
    } else {
        {envelope_risk: "advisory: the body names a loop-asset or external-tool surface the implementer fs envelope hides — verify the PRIMARY fix surface is repo-visible (lib/ apps/ docs/ web) before claiming; loop-asset-only or external-tool seeds are DIRECT WORK (skip, journal the classification)"}
    })
    # fabro-d95c: the Basis-commit legend keeps the table
    # self-interpreting (fabro-ead4 legend contract): an unreachable sha
    # is NORMAL for squash-stranded basis commits, never a stale verdict.
    let basis_legend = {basis_commit: "advisory: the body's Basis-line commit sha. basis_commit_resolves=false means the commit is unreachable in this object DB (squash-merged run branches strand their basis commits) — NOT a stale-basis verdict: adjudicate the basis via its files/worktree, never probe for the sha; null = no sha in body or tooling fail-open"}
    # fabro-4401: a `duplicate` is ALWAYS advisory-only — never a mechanical
    # close. Matches whose only seed-id reference sits in journal-style
    # bullet lines of a landed commit's message are mention-only and are
    # downgraded before the verdict, so they surface as this count instead
    # of a false 'duplicate' (incident: cherry-pick a7f7b2d4b8 vs run
    # 01M3YWBVKHAC8EHPYWM9BQBXZ3). closure=foreign implementation matches
    # still yield 'duplicate' — advisory input, the planner judges
    # acceptance criteria per the two-branch rule.
    let mention_legend = {mention_only_matches: "advisory: landed commits whose only seed-id reference sits in journal-style message bullets — mentions, never landed implementations; they cannot produce a 'duplicate' verdict"}
    # fabro-a8d5 stranded_implementation legend: maps every salvage
    # pointer problem to its planner action so the table stays
    # self-interpreting (the fabro-ead4 legend contract).
    let salvage_legend = {stranded_implementation: "advisory: salvage/stranded-implementation pointers verified at claim time — salvage_ok false names commit_missing (pointer cites a commit that no longer resolves), already_in_base (the salvage landed on the merge target; the ALREADY LANDED two-branch-rule arm owns the seed), branch_missing (salvage-source run branch no longer fetched), or pointer_missing (salvage-source run branch cited without the canonical 'salvage: git show <commit> on branch <branch>' bullet — resolve the implementer commit BEFORE shipping the brief)"}
    let report = {mode: $mode,
                  run_id: ($run_id | default null),
                  candidates: ($verdicts | enumerate | each {|e| row $e.item ($cdesc | get -o $e.index | default "") ($SCRIPT_DIR | path join '../../../..') ($inflight.claims | default []) $base ($inflight.fetched? | default false) $assignee}),
                  legend: ($legend | merge $envelope_legend | merge $basis_legend | merge $mention_legend | merge $salvage_legend),
                  degraded_reason: (if ($degraded_reason | is-empty) { null } else { $degraded_reason }),
                  in_flight_note: ($inflight.note? | default null)}
    {"outcome": "succeeded",
     "preferred_next_label": "Preflight done",
     "context_updates": {"output.preflight": $report}} | to json --raw | print
}
