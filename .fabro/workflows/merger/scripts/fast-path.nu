#!/usr/bin/env nu
# The merger fast path (ADR-0039 stage E, denkhaus/work#1107): the
# LLM-free batch sweep over the closable queue. One command stage walks
# the WHOLE worklist — every terminal develop run the adjudicator
# marked merge:enabled without a settled closable verdict — merges the
# clean mass through the door, smokes every integrated tree (the
# grilled cheap verification: cargo check on the merged line), and
# marks the verdict; conflicts and parks route to the agent pass as
# packets (the evidence-to-reviewer pipe: scripts merge, the agent
# resolves).
#
# THE SMOKE (grilled 2026-11-02, cheap_smoke): after each merge the
# sandbox fetches the line branch and checks out the merged tip
# in-place (the shared target/ keeps the cache warm — a fresh worktree
# would pay a cold build per run), cargo check --workspace runs, and
# the original checkout is restored. A green smoke marks
# closable:enabled; a red smoke parks (the integration broke the line —
# the operator sees the parked run, the next pass re-smokes after a
# fix or revert).
#
# PARKED runs re-enter every pass (only enabled and refused free a
# run): they route to the agent pass — an open escalation is a judgment
# the fast path cannot resolve.
#
# FACT FAILURE DISCIPLINE (the standing rule): a door call that fails
# FAILS THE STAGE — exit non-zero, the engine retries; never absorb a
# failed read or write. The ONE exception is the door's own conflict
# signal (the merge_conflict refusal): that is a packet, not a failure.

use ../../../scripts/lib/work_channel.nu [
    server, run-merge-argv, cli, read-queue-hard, mark-run-hard, emit-envelope, title-own-run
]

const CALLER = 'merger-fast-path'
const FIELD = 'closable'
const WORKFLOW = 'develop'

# One verdict write through the door (HARD: a failed mark fails the
# stage — the engine retries the whole sweep; marks are idempotent at
# the fold, a re-sweep is gap-safe).
# The verdict write (HARD: the wrapper is the shared policy, #1132);
# this alias carries the script's FIELD.
def mark-run [run_id: string, verdict: string]: nothing -> nothing {
    mark-run-hard $CALLER $FIELD $run_id $verdict
}

# The cheap integration smoke (grilled): fetch the line, check out the
# merged tip in-place, cargo check the workspace, restore the original
# checkout. The restore keys on the HEAD SHA (a branch NAME round-trips
# dirty through nu strings and once detached the session's HEAD — the
# 2026-11-03 incident this comment memorializes). Returns 'green' or
# 'red' — transport failures are RED too (fail-closed: an
# unverifiable tree never enables a close).
def smoke-merged-tree []: nothing -> string {
    let original = (do { git rev-parse HEAD } | complete)
    if $original.exit_code != 0 {
        print -e $"($CALLER): smoke cannot resolve the checkout: ($original.stderr | str trim | str substring 0..200)"
        return 'red'
    }
    let original_sha = ($original.stdout | str trim)
    let fetch = (do { git fetch origin denkhaus } | complete)
    if $fetch.exit_code != 0 {
        print -e $"($CALLER): smoke fetch failed: ($fetch.stderr | str trim | str substring 0..200)"
        return 'red'
    }
    let checkout = (do { git checkout -q FETCH_HEAD } | complete)
    if $checkout.exit_code != 0 {
        print -e $"($CALLER): smoke checkout failed: ($checkout.stderr | str trim | str substring 0..200)"
        return 'red'
    }
    let check = (do { cargo check --workspace --quiet } | complete)
    # Restore the engine's checkout WHATEVER the check said — the SHA is
    # the unambiguous target (a branch name would detach a session that
    # started on one).
    let restore = (do { git checkout -q $original_sha } | complete)
    if $restore.exit_code != 0 {
        print -e $"($CALLER): smoke restore failed: ($restore.stderr | str trim | str substring 0..200)"
    }
    if $check.exit_code == 0 { 'green' } else {
        print -e $"($CALLER): smoke RED — the integrated tree does not check: ($check.stderr | str trim | str substring 0..300)"
        'red'
    }
}

# Pure: the provider-agnostic repository spelling of a forge origin URL
# (`https://github.com/owner/name.git` and `git@host:owner/name.git` both
# give `owner/name`; the label door takes exactly that spelling).
export def origin-repo [url: string]: nothing -> any {
    let url = ($url | str trim | str replace -r '\.git$' '')
    if ($url | parse -r '^[a-z]+://[^/]+/(?<repo>[^/]+/[^/]+)$' | get -o repo.0) != null {
        $url | parse -r '^[a-z]+://[^/]+/(?<repo>[^/]+/[^/]+)$' | get repo.0
    } else if ($url | parse -r '^[^:/@]+@[^:]+:(?<repo>[^/]+/[^/]+)$' | get -o repo.0) != null {
        $url | parse -r '^[^:/@]+@[^:]+:(?<repo>[^/]+/[^/]+)$' | get repo.0
    } else {
        null
    }
}

# The label arm (ADR-0040's session half, work#1205): open pull requests
# of the checkout's repository carrying the merge:enabled label — session
# work integrating through the same door the run sweep uses. Run pull
# requests carry no label, so the worklists cannot overlap. Version
# skew: until the server ships the label door, the list call fails and
# the arm SKIPS with a warning (never a stage failure — the run sweep
# above already did its job).
def label-arm [srv: string, smoke: bool]: nothing -> record {
    let origin = (do { git remote get-url origin } | complete)
    if $origin.exit_code != 0 {
        print -e $"($CALLER): label arm cannot resolve the checkout origin — skipping"
        return {listed: 0, merged: [], red_smoke: []}
    }
    let repo = (origin-repo $origin.stdout)
    if $repo == null {
        print -e $"($CALLER): label arm cannot parse the origin '($origin.stdout | str trim)' — skipping"
        return {listed: 0, merged: [], red_smoke: []}
    }

    let list_res = (cli $CALLER ['pr' 'list' '--server' $srv '--repo' $repo '--label' 'merge:enabled' '--json'])
    if $list_res.exit_code != 0 {
        print -e $"($CALLER): label door unavailable on this server — skipping the session arm: ($list_res.stderr | str trim | str substring 0..160)"
        return {listed: 0, merged: [], red_smoke: []}
    }
    let prs = ($list_res.stdout | from json | get -o data | default [])
    if ($prs | is-empty) {
        return {listed: 0, merged: [], red_smoke: []}
    }
    print $"($CALLER): label arm: ($prs | length) session pull request\(s\) admitted"

    mut merged = []
    mut red = []
    for pr in $prs {
        let merge_res = (cli $CALLER ['pr' 'merge-by-id' $pr.id '--server' $srv '--repo' $repo '--method' 'merge' '--json'])
        if $merge_res.exit_code != 0 {
            print -e $"($CALLER): label merge failed for #($pr.id): ($merge_res.stderr | str trim | str substring 0..200)"
            continue
        }
        print $"($CALLER): merged session pull request #($pr.id) \(($pr.title | str substring 0..80)\)"
        # The same grilled cheap smoke as the run sweep: the integrated
        # tree must check. A red smoke cannot park (no run) — it reports
        # loudly here and in the title; the operator reverts.
        let smoke_verdict = (if $smoke { 'green' } else { smoke-merged-tree })
        if $smoke_verdict == 'green' {
            $merged = ($merged | append $pr.id)
        } else {
            $red = ($red | append $pr.id)
        }
    }
    {listed: ($prs | length), merged: $merged, red_smoke: $red}
}

def main [--smoke]: nothing -> nothing {
    let srv = (server $CALLER)
    # The merger's worklist (work#1126): the closable anti-join
    # RESTRICTED to merge:enabled runs — a merge:pending run cannot kill
    # the sweep at the door and a merge:refused run cannot become an
    # immortal parked packet.
    let entries = (read-queue-hard $CALLER $FIELD $WORKFLOW '' 'merge=enabled')

    mut packets = []
    mut merged_enabled = []
    mut smoke_parked = []
    mut malformed_parked = []
    for entry in $entries {
        let run_id = $entry.run.id
        let has_pr = ($entry.run | get -o pull_request | default null) != null

        if $entry.verdict == "parked" {
            # An open escalation re-enters: the agent pass re-engages
            # with the conflict's context.
            $packets = ($packets | append {run_id: $run_id, verdict: "parked", kind: "parked", pull_request: ($entry.run | get -o pull_request | default null)})
            continue
        }

        if not $has_pr {
            # Malformed: the adjudicator enabled a PR-less run (the
            # pass-one fast path refuses those). Park for the operator.
            mark-run $run_id "parked"
            $malformed_parked = ($malformed_parked | append $run_id)
            continue
        }

        # The merge door. A conflict refusal is a packet; anything else
        # non-zero is a HARD failure.
        let merge_res = (cli $CALLER (run-merge-argv $srv $run_id))
        if $merge_res.exit_code != 0 {
            if ($merge_res.stderr | str contains '[merge_conflict]') {
                $packets = ($packets | append {run_id: $run_id, verdict: "pending", kind: "conflict", pull_request: ($entry.run | get -o pull_request | default null)})
                continue
            }
            print -e $"($CALLER): run merge failed for ($run_id): ($merge_res.stderr | str trim | str substring 0..300)"
            exit 1
        }
        print $"($CALLER): merged ($run_id) \(($merge_res.stdout | str trim)\)"

        # The grilled cheap smoke on the integrated tree. The --smoke
        # fixture drive skips the tree work (no checkout exists in a
        # unit smoke; the transport arms are pinned by the conflicted
        # fixture instead) and treats the integration as green.
        let smoke_verdict = (if $smoke { 'green' } else { smoke-merged-tree })
        if $smoke_verdict == 'green' {
            mark-run $run_id "enabled"
            $merged_enabled = ($merged_enabled | append $run_id)
        } else {
            mark-run $run_id "parked"
            $smoke_parked = ($smoke_parked | append $run_id)
        }
    }

    # The label arm (work#1205): session pull requests admitted by the
    # merge:enabled label integrate through the same door.
    let label = (label-arm $srv $smoke)

    # The pass title (#1159 follow-through): the sweep shape lands on
    # the board tile. Advisory (work_channel::title-own-run).
    title-own-run $CALLER $"Merger: merged \(($merged_enabled | length)\) runs + \(($label.merged | length)\) session PRs with green smoke; parked \(($smoke_parked | length)\) red-smoke + \(($malformed_parked | length)\) malformed + \(($label.red_smoke | length)\) red session smokes; \(($packets | length)\) packet\(s\) for the agent pass"
    print $"($CALLER): merged \(($merged_enabled | length)\) runs with green smoke; session arm merged \(($label.merged | length)\) of \(($label.listed)\) \(($label.red_smoke | length)\) red\); parked \(($smoke_parked | length)\) on red smoke and \(($malformed_parked | length)\) malformed; \(($packets | length)\) packet\(s\) for the agent pass"
    let integration = {
        packets: $packets,
        merged_enabled: $merged_enabled,
        smoke_parked: $smoke_parked,
        malformed_parked: $malformed_parked,
        label_arm: $label,
    }
    let label = (if ($packets | is-empty) { "Queue empty" } else { "Runs pending integration" })
    emit-envelope $label 'integration' $integration $smoke
}
