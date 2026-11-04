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
    server, run-queue-argv, entries-from, run-mark-argv, run-merge-argv, cli
]

const CALLER = 'merger-fast-path'
const FIELD = 'closable'
const WORKFLOW = 'develop'

# One verdict write through the door (HARD: a failed mark fails the
# stage — the engine retries the whole sweep; marks are idempotent at
# the fold, a re-sweep is gap-safe).
def mark-run [run_id: string, verdict: string]: nothing -> nothing {
    let srv = (server $CALLER)
    let res = (cli $CALLER (run-mark-argv $srv $run_id $FIELD $verdict))
    if $res.exit_code != 0 {
        print -e $"($CALLER): run mark failed for ($run_id) \(($verdict)\): ($res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    print $"($CALLER): marked ($run_id) \($FIELD = ($verdict)\)"
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

def main [--smoke]: nothing -> nothing {
    let srv = (server $CALLER)
    # The merger's worklist (work#1126): the closable anti-join
    # RESTRICTED to merge:enabled runs — a merge:pending run cannot kill
    # the sweep at the door and a merge:refused run cannot become an
    # immortal parked packet.
    let queue_res = (cli $CALLER (run-queue-argv $srv $FIELD $WORKFLOW '' 'merge=enabled'))
    if $queue_res.exit_code != 0 {
        print -e $"($CALLER): run queue read failed: ($queue_res.stderr | str trim | str substring 0..240)"
        exit 1
    }
    let entries = (entries-from $queue_res.stdout)
    print $"($CALLER): worklist holds ($entries | length) run\(s\) on \($FIELD)/($WORKFLOW)\)"

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

    print $"($CALLER): merged \(($merged_enabled | length)\) with green smoke; parked \(($smoke_parked | length)\) on red smoke and \(($malformed_parked | length)\) malformed; \(($packets | length)\) packet\(s\) for the agent pass"
    let integration = {
        packets: $packets,
        merged_enabled: $merged_enabled,
        smoke_parked: $smoke_parked,
        malformed_parked: $malformed_parked,
    }
    if $smoke {
        # The smoke drive: print the packet record for assertion, no
        # routing envelope (no engine consumes this output).
        $integration | to json --raw | print
        return
    }
    let label = (if ($packets | is-empty) { "Queue empty" } else { "Runs pending integration" })
    {preferred_next_label: $label, context_updates: {output.integration: $integration}} | to json --raw | print
}
