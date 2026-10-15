#!/usr/bin/env nu
# Run a workflow end to end and integrate the result
# (thin launcher: `just run <workflow>`).
#
# Pipeline:
#   1. guards: clean worktree on a world branch (never fabro/run/*)
#   2. fabro create <workflow> [--goal] --json   -> run id
#      (or --adopt <run-id> to resume with an existing run)
#   3. fabro start <id> + fabro attach <id>      -> live output
#   4. fabro wait <id> --json                    -> status/reason truth
#      then resolve the run PR: the wait JSON when it carries one,
#      else `wait-for-late-pr` (bounded gh poll — the platform may
#      create the PR seconds AFTER terminal status); orchestration
#      runs (PR-less workflows, by slug) fast-exit here instead
#   5. integrate (fetch origin first), via the defs below:
#        - `post-lab-check`: post the required `lab-check` status
#          LOCALLY on the run branch head (variant B, fabro-ab2c:
#          no Actions runner involved; the lab-check.yml workflow is
#          disabled/commented in the tree)
#        - `wait-auto-merge` + `wait-pending-gate`: GitHub auto-merge
#          merges the PR -> run branch contained in origin/<branch>
#          -> pull --ff-only
#        - else `emergency-squash`: squash-merge fabro/run/<id> into
#          <branch>, push, sweep the leftover PR (fallback when
#          auto-merge cannot engage; the leftover PR on GitHub can be
#          closed as already-integrated)
#
# No ask-based improve review anymore (ADR-0015): the revisor workflow
# owns run revisioning on the merged world — it embeds the improve
# question itself and files seeds with a basis line.
#
# Any failure prints an ALARM block and exits 1; nothing is merged for a
# red run.
#
# External calls follow the repo style (qualitygate.nu): literal external
# commands in `do { ^cmd ... } | complete`, exit_code checked. No generic
# arg-spread wrapper — rest params would swallow --flags meant for the
# external command.

# Line server (fabro-e62d): the manual develop era targets mirtuell — the
# local 127.0.0.1 stack is for TESTS only and a silent default there costs a
# dead start (connection refused at fabro create). Override with FABRO_SERVER.
const SERVER_DEFAULT = 'https://mirtuell.net'
const GITHUB_REPO = 'denkhaus/fabro'  # lab world repo (variant B status posts)

# ── PR-integration tuning knobs (fabro-164b, ADR-0028) ──────────────
# One place for every timing the PR-integration half was re-tuned on
# (five structural touches in 12 days). Retune HERE, never inline in
# the defs below.
const PR_POLL_ROUNDS = 60          # late-PR positive lookup: 60 x 5s = 5 min
                                  # (fabro-767b: PRs observed ~2-3 min after
                                  # terminal; 90s was too short and sent a
                                  # healthy integration to emergency squash)
const PR_POLL_SLEEP = 5sec
const MERGE_DEADLINE_ROUNDS = 120  # auto-merge wait: 120 x 15s = 30 min
                                  # (fabro-767b: the dogfood-gate takes
                                  # ~25 min; a shorter deadline squashes
                                  # while the gate is still RUNNING)
const MERGE_POLL_SLEEP = 15sec
const PENDING_GUARD_ROUNDS = 60    # pre-squash pending-checks guard:
                                  # 60 x 15s = 15 min extra gate patience
const GUARD_SLEEP = 15sec

# Terminal failure: loud ALARM block on stderr, exit 1.
def fail [msg: string]: nothing -> nothing {
    print -e ''
    print -e $"╔══ ALARM: ($msg) ══╗"
    print -e '╚══════════════════════════════════════════════════════════════╝'
    exit 1
}

# Check an external result; on failure exit via `fail` with stderr detail.
def ok [result: record, what: string]: nothing -> record {
    if $result.exit_code != 0 {
        let detail = ($result.stderr | str trim | default $result.stdout | str trim)
        fail $"($what) failed: ($detail)"
    }
    $result
}

# ── PR-integration defs (fabro-164b extraction) ─────────────────────

# gh pr list for the run branch in a given state; [] on any gh/parse
# failure (fail-open: a broken poll never fabricates a merge).
def run-prs [run_id: string, state: string]: nothing -> list {
    let listed = (do {
        ^gh pr list --repo $GITHUB_REPO --head $"fabro/run/($run_id)" --state $state --json number --limit 1
    } | complete)
    if $listed.exit_code != 0 { return [] }
    try { $listed.stdout | from json } catch { [] }
}

# GitHub's own PR state is the authoritative merged signal (fabro-e62d:
# tree equality alone missed a landed squash merge — observed PR #349
# merged while the poll kept reporting not landed).
def run-pr-merged [run_id: string]: nothing -> bool {
    ((run-prs $run_id 'merged') | length) > 0
}

# Tree-equality merge detection — not commit ancestry: GitHub auto-merge
# SQUASHES the run branch into the base (run 01M11P68SHFS: merge commit
# 5f953db8, one parent), so the run-branch tip is never an ancestor of
# origin/<branch> after integration.
def trees-equal [run_branch: string, base: string]: nothing -> bool {
    ((do { git diff --quiet $run_branch $base } | complete).exit_code == 0)
}

def fast-forward [branch: string]: nothing -> nothing {
    let _ = (ok (do { git pull --ff-only origin $branch } | complete) 'git pull')
}

# Bounded positive lookup for a PR the wait JSON did not carry: poll
# `gh pr list --head fabro/run/<id>` for a short window before
# concluding anything (fabro-4acd: the PR appeared ~20s after terminal).
# On a no-hit, still continue — the run-branch check in main decides
# loudly whether anything can be integrated.
def wait-for-late-pr [run_id: string]: nothing -> nothing {
    mut pr_found = false
    print $"run_workflow: wait JSON lacks pull_request — polling gh for the run PR for ($PR_POLL_ROUNDS * 5) sec"
    for _ in 1..$PR_POLL_ROUNDS {
        sleep $PR_POLL_SLEEP
        if ((run-prs $run_id 'open') | length) > 0 {
            $pr_found = true
            break
        }
    }
    if $pr_found {
        print 'run_workflow: run PR found after terminal (created late) — continuing integration'
    } else {
        print 'run_workflow: WARN no run PR seen within the poll window — continuing; the run-branch check below decides'
    }
}

# Variant B (fabro-ab2c): post the required `lab-check-local` context
# locally on the run branch head. The name deliberately differs from the
# workflow's `lab-check`: GitHub auto-resolves a plain context string to
# the app that last reported a check run of that name (Actions, app
# 15368) — pinning it so a commit status can never satisfy it.
# `lab-check-local` has no app history and stays plain (commit statuses
# count). GitHub auto-merge (enabled by the platform at PR creation,
# project.toml auto_merge = true) merges the PR without any Actions
# runner. Failure is a WARN, not fatal — the squash fallback still
# integrates the work.
def post-lab-check [run_sha: string]: nothing -> nothing {
    let posted = (do { ^gh api $"repos/($GITHUB_REPO)/statuses/($run_sha)" -f state=success -f context=lab-check-local -f description='local lab-check (run terminal-succeeded)' } | complete)
    if $posted.exit_code != 0 {
        let detail = ($posted.stderr | str trim | default $posted.stdout | str trim)
        print $"run_workflow: WARN local lab-check post failed — auto-merge cannot engage, squash fallback will integrate: ($detail)"
    }
}

# Auto-merge is the PRIMARY integration path (user directive 2026-09-05:
# local squash is only an emergency workaround): the Dogfood Gate
# (path-adaptive CI) usually completes in seconds for markdown-only run
# PRs and in minutes for code PRs, so wait for the REAL GitHub merge
# instead of racing it locally. Bounded by MERGE_DEADLINE_ROUNDS.
def wait-auto-merge [run_id: string, run_branch: string, branch: string]: nothing -> bool {
    mut auto_merged = false
    for _ in 1..$MERGE_DEADLINE_ROUNDS {
        sleep $MERGE_POLL_SLEEP
        let _ = (do { git fetch origin $branch } | complete)
        $auto_merged = (trees-equal $run_branch $"origin/($branch)")
        if not $auto_merged {
            $auto_merged = (run-pr-merged $run_id)
        }
        if $auto_merged { break }
    }
    $auto_merged
}

# fabro-767b pre-squash guard: if an OPEN run PR exists and its checks
# are still pending, the gate is doing its job — keep waiting (bounded)
# instead of squashing past a running gate. Returns whether auto-merge
# landed during the guard.
def wait-pending-gate [run_id: string]: nothing -> bool {
    mut guard = 0
    mut auto_merged = false
    while $guard < $PENDING_GUARD_ROUNDS {
        let prs = (run-prs $run_id 'open')
        if ($prs | length) == 0 { break }
        let checks = (do {
            ^gh pr checks --repo $GITHUB_REPO ($prs | first | get number) --json state,bucket
        } | complete)
        let pend = (try { $checks.stdout | from json | where bucket == 'pending' | length } catch { 1 })
        if $pend == 0 { break }
        print $"run_workflow: run PR exists with pending checks — waiting out the gate \(fabro-767b guard, round ($guard)\)"
        sleep $GUARD_SLEEP
        $auto_merged = (run-pr-merged $run_id)
        if $auto_merged { break }
        $guard = $guard + 1
    }
    $auto_merged
}

# EMERGENCY fallback (fabro-ab2c, provisional auto-merge): squash-merge
# the run branch into the world branch and push. Kept because it has
# fired productively (the PR #357 incident integrated via this path).
# Also recovers a late auto-merge landing between poll and push, and
# sweeps the leftover PR + run branch so the push-gate inherits no
# ghost.
def emergency-squash [workflow: string, run_id: string, run_branch: string, branch: string]: nothing -> nothing {
    print $"run_workflow: WARN auto-merge did not land within ($MERGE_DEADLINE_ROUNDS * 15) sec — EMERGENCY squash fallback engages \(dogfood-gate stuck? check the PR checks\)"
    print $"run_workflow: squash-merging ($run_branch) into ($branch) \(provisional auto-merge, fabro-ab2c\)"
    ok (do { git merge --squash $run_branch } | complete) 'squash merge'
    let nothing_staged = ((do { git diff --cached --quiet } | complete).exit_code == 0)
    if $nothing_staged {
        print 'run_workflow: run branch carries no tree changes — nothing to commit'
    } else {
        ok (do { git commit -m $"($workflow): run ($run_id)" } | complete) 'git commit'
        let pushed = (do { git push origin $branch } | complete)
        if $pushed.exit_code != 0 {
            # A late auto-merge can land between the poll above and
            # this push: origin moved ahead, the push rejects
            # non-fast-forward. If the remote tree now equals the run
            # branch's tree, GitHub integrated the same work — the
            # local squash is redundant. Drop it, sync to origin, and
            # CONTINUE (integration already happened); only a push
            # failure without a landed merge stays fatal.
            let _ = (do { git fetch origin $branch } | complete)
            if (trees-equal $run_branch $"origin/($branch)") {
                print 'run_workflow: auto-merge landed during squash fallback — syncing to origin, continuing'
                ok (do { git reset --hard $"origin/($branch)" } | complete) 'git reset'
            } else {
                let detail = ($pushed.stderr | str trim | default $pushed.stdout | str trim)
                fail $"git push failed: ($detail)"
            }
        }
    }

    # fabro-767b sweep: a PR that appeared late (or never merged) is
    # redundant once the squash carried the work — close it as
    # already-integrated and delete the run branch so the push-gate
    # inherits no ghost (PR #357 incident).
    let lprs = (run-prs $run_id 'open')
    if ($lprs | length) > 0 {
        let lnum = ($lprs | first | get number)
        print $"run_workflow: closing leftover run PR #($lnum) as already-integrated (fabro-767b sweep)"
        let _ = (do {
            ^gh pr close ($lnum) --repo $GITHUB_REPO --comment $"Already integrated via emergency squash into ($branch) — the PR appeared late or its gate never concluded (fabro-767b)."
        } | complete)
    }
    let _ = (do { git push origin --delete $"fabro/run/($run_id)" } | complete)
}

def main [
    workflow: string = 'develop'  # workflow name (as `fabro run` accepts)
    --goal (-g): string           # optional goal override (name the seed!)
    --branch (-b): string         # world/base branch; default: current
    --adopt (-a): string          # adopt an existing run id (skip create/start)
    --timeout-min (-t): int = 90  # minutes to wait for the run
    --environment (-e): string = 'toolchain'  # server-managed environment
                                         # (fabro-03ef: v0.345 intent runs drop
                                         # project-level [run] settings; the
                                         # intent environment override outranks
                                         # the workflow.toml pin). Default is
                                         # toolchain on the merged world: the
                                         # develop tester needs just/nu/seeds,
                                         # which fabro-runner:mise LACKS
                                         # (cycle 1, 2026-09-05: 'just: command
                                         # not found' 3x deterministic).
]: nothing -> nothing {
    let fabro_bin = ($env.FABRO_BIN? | default $"($env.HOME)/.fabro/bin/fabro")
    let server = ($env.FABRO_SERVER? | default $SERVER_DEFAULT)
    # fabro-e62d: quoting-proof goal source — just interpolates {{ args }} raw
    # into the recipe line, so a --goal with spaces/parens breaks bash or nu.
    # FABRO_RUN_GOAL passes through every layer un-reparsed.
    let goal = (if ($goal | is-empty) { $env.FABRO_RUN_GOAL? | default '' } else { $goal })

    # ── 1. guards ────────────────────────────────────────────────────
    let current = ((do { git branch --show-current } | complete).stdout | str trim)
    if ($current | is-empty) {
        fail 'not on a branch (detached head?)'
    }
    let branch = (if ($branch | is-empty) { $current } else { $branch })
    if ($branch | str starts-with 'fabro/') {
        fail $"refusing to integrate into run branch '($branch)' — switch to a world branch"
    }
    # Run 01M4GXA88RC2 (2026-10-09): a fire whose ambient branch was the
    # GitButler workspace resolved the stale origin/gitbutler/workspace shadow
    # (a 2026-10-03 tree + tracker) and died at the planner's fs-envelope
    # guard. The workspace stack is never the published line.
    if ($branch | str starts-with 'gitbutler/') {
        fail $"refusing '($branch)' as fire source — a GitButler workspace branch is never the published line; a stale origin shadow of it exists. Pass --branch with a world branch, or fire from a plain checkout."
    }
    let dirty = ((do { git status --porcelain } | complete).stdout | str trim)
    if not ($dirty | is-empty) {
        fail $"worktree is dirty — commit or stash first:\n($dirty)"
    }
    print $"run_workflow: branch=($branch) workflow=($workflow)"

    # ── 2. create (or adopt) ─────────────────────────────────────────
    let run_id = (if not ($adopt | is-empty) {
        print $"run_workflow: adopting run ($adopt)"
        $adopt
    } else {
        # Workflow source + target come from the PUSHED branch (fabro-ed65 era,
        # observed 2026-10-09): inside a GitButler workspace the local HEAD is
        # the never-pushed workspace commit, so a bare `fabro create` refuses
        # with "the exact local Git commit could not be made available from the
        # canonical GitHub origin". Pinning both to origin/<branch> also makes a
        # manual fire reproduce exactly what the line published — uncommitted
        # workflow edits never ride a fire (the dirty-worktree guard above
        # already enforces that earlier).
        let source = [
            '--workflow-repo' $GITHUB_REPO
            '--workflow-ref' $branch
            '--target' $"($GITHUB_REPO)@($branch)"
        ]
        let created = (if ($goal | is-empty) {
            do { ^$fabro_bin create $workflow ...$source --environment $environment --json --server $server } | complete
        } else {
            do { ^$fabro_bin create $workflow --goal $goal ...$source --environment $environment --json --server $server } | complete
        })
        ok $created 'fabro create'
        let id = ($created.stdout | from json | get -o run_id | default '')
        if ($id | is-empty) {
            fail $"fabro create returned no run_id: ($created.stdout | str trim)"
        }
        $id
    })
    print $"run_workflow: run id ($run_id)"

    # ── 3. start + attach (live output) ──────────────────────────────
    if ($adopt | is-empty) {
        let started = (do { ^$fabro_bin start $run_id --server $server } | complete)
        ok $started 'fabro start'
    }
    # attach streams live output; a non-zero exit is expected on failed
    # runs and is NOT the verdict — step 4 owns that decision.
    try { ^$fabro_bin attach $run_id --server $server }

    # ── 4. wait: the status truth ────────────────────────────────────
    let timeout_sec = ($timeout_min * 60)
    let waited = (do {
        ^$fabro_bin wait $run_id --json --timeout $timeout_sec --server $server
    } | complete)
    # fabro wait may exit non-zero when the RUN failed while still
    # emitting the terminal status JSON — that is the verdict we want
    # below, not an empty wrapper alarm (cycle 1, 2026-09-05). Only a
    # result without parseable JSON is a tool failure.
    let info = (try { $waited.stdout | from json } catch { null })
    if $info == null {
        ok $waited 'fabro wait'
    }

    let status = ($info | get -o status | default '')
    let reason = ($info | get -o reason | default '')
    print $"run_workflow: terminal status ($status) ($reason)"
    if $status != 'succeeded' {
        fail $"run ($run_id) ended ($status) ($reason) — nothing integrated; inspect ($server)/runs/($run_id)"
    }

    # Orchestration runs disable PRs: no PR means nothing to auto-merge —
    # skip the integration wait instead of polling a merge that cannot
    # happen (the journal stays on the run branch; the UI and Slack carry
    # the outcomes). PR-less workflows are distinguished by SLUG, never by
    # PR absence: the wait JSON's pull_request field is only populated
    # while the wait event stream saw the PR, and the platform can create
    # the PR ~seconds AFTER terminal status (fabro-4acd race) — treating
    # absence as orchestration silently skipped integration.
    const prless_workflows = ['conductor']
    if $workflow in $prless_workflows {
        print $"run_workflow: orchestration workflow ($workflow) disables PRs - nothing to integrate"
        exit 0
    }
    let has_pr = (($info | get -o pull_request | default null) != null)
    if not $has_pr {
        wait-for-late-pr $run_id
    }

    # ── 5. integrate the run branch ──────────────────────────────────
    ok (do { git fetch origin } | complete) 'git fetch'
    let run_branch = $"origin/fabro/run/($run_id)"
    let has_branch = ((do { git rev-parse --verify $"refs/remotes/($run_branch)" } | complete).exit_code == 0)
    if not $has_branch {
        if $reason == 'publish_blocked' {
            fail 'publish blocked and the run branch was NOT pushed — work lives in the server checkpoint; fix credentials and retry'
        }
        fail $"run branch ($run_branch) not found — nothing to integrate"
    }
    let run_sha = ((do { git rev-parse $run_branch } | complete).stdout | str trim)
    post-lab-check $run_sha
    if (wait-auto-merge $run_id $run_branch $branch) {
        print 'run_workflow: auto-merge landed — fast-forward pull'
        fast-forward $branch
    } else if (wait-pending-gate $run_id) {
        print 'run_workflow: auto-merge landed during the guard — fast-forward pull'
        fast-forward $branch
    } else {
        emergency-squash $workflow $run_id $run_branch $branch
    }

    print $"run_workflow: done — run ($run_id) integrated"
}
