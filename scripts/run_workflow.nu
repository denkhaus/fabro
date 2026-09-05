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
#   5. integrate (fetch origin first):
#        - post the required `lab-check` status LOCALLY on the run branch
#          head (variant B, fabro-ab2c: no Actions runner involved; the
#          lab-check.yml workflow is disabled/commented in the tree)
#        - GitHub auto-merge merges the PR -> run branch contained in
#          origin/<branch> -> pull --ff-only
#        - else squash-merge fabro/run/<id> into <branch>, push
#          (fallback when auto-merge cannot engage; the leftover PR on
#          GitHub can be closed as already-integrated)
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

const SERVER_DEFAULT = 'http://127.0.0.1:32276'
const GITHUB_REPO = 'denkhaus/fabro'  # lab world repo (variant B status posts)

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

def main [
    workflow: string = 'develop'  # workflow name (as `fabro run` accepts)
    --goal (-g): string           # optional goal override (name the seed!)
    --branch (-b): string         # world/base branch; default: current
    --adopt (-a): string          # adopt an existing run id (skip create/start)
    --timeout-min (-t): int = 90  # minutes to wait for the run
    --environment (-e): string = 'mise'  # server-managed environment (fabro-03ef:
                                         # v0.345 intent runs drop project-level
                                         # [run] settings; the intent environment
                                         # override outranks the workflow.toml pin)
]: nothing -> nothing {
    let fabro_bin = ($env.FABRO_BIN? | default $"($env.HOME)/.fabro/bin/fabro")
    let server = ($env.FABRO_SERVER? | default $SERVER_DEFAULT)

    # ── 1. guards ────────────────────────────────────────────────────
    let current = ((do { git branch --show-current } | complete).stdout | str trim)
    if ($current | is-empty) {
        fail 'not on a branch (detached head?)'
    }
    let branch = (if ($branch | is-empty) { $current } else { $branch })
    if ($branch | str starts-with 'fabro/') {
        fail $"refusing to integrate into run branch '($branch)' — switch to a world branch"
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
        let created = (if ($goal | is-empty) {
            do { ^$fabro_bin create $workflow --environment $environment --json --server $server } | complete
        } else {
            do { ^$fabro_bin create $workflow --goal $goal --environment $environment --json --server $server } | complete
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
    ok $waited 'fabro wait'
    let info = ($waited.stdout | from json)

    let status = ($info | get -o status | default '')
    let reason = ($info | get -o reason | default '')
    print $"run_workflow: terminal status ($status) ($reason)"
    if $status != 'succeeded' {
        fail $"run ($run_id) ended ($status) ($reason) — nothing integrated; inspect ($server)/runs/($run_id)"
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
    # Variant B (fabro-ab2c): post the required `lab-check-local`
    # context locally on the run branch head. The name deliberately
    # differs from the workflow's `lab-check`: GitHub auto-resolves a
    # plain context string to the app that last reported a check run of
    # that name (Actions, app 15368) — pinning it so a commit status can
    # never satisfy it. `lab-check-local` has no app history and stays
    # plain (commit statuses count). GitHub auto-merge (enabled by the
    # platform at PR creation, project.toml auto_merge = true) merges the
    # PR without any Actions runner. Failure is a WARN, not fatal — the
    # squash fallback below still integrates the work.
    let run_sha = ((do { git rev-parse $run_branch } | complete).stdout | str trim)
    let posted = (do { ^gh api $"repos/($GITHUB_REPO)/statuses/($run_sha)" -f state=success -f context=lab-check-local -f description='local lab-check (run terminal-succeeded)' } | complete)
    if $posted.exit_code != 0 {
        let detail = ($posted.stderr | str trim | default $posted.stdout | str trim)
        print $"run_workflow: WARN local lab-check post failed — auto-merge cannot engage, squash fallback will integrate: ($detail)"
    }

    # Auto-merge engages within seconds of the status; poll briefly so
    # the ff-pull path wins over the local squash fallback.
    #
    # Detection is TREE EQUALITY, not commit ancestry: GitHub auto-merge
    # SQUASHES the run branch into the base (run 01M11P68SHFS: merge
    # commit 5f953db8, one parent), so the run-branch tip is never an
    # ancestor of origin/<branch> after integration — an is-ancestor
    # check can never see the landed merge and always falls into the
    # squash fallback (push race, ALARM exit).
    mut auto_merged = false
    for _ in 1..9 {
        sleep 10sec
        let _ = (do { git fetch origin $branch } | complete)
        $auto_merged = ((do {
            git diff --quiet $run_branch $"origin/($branch)"
        } | complete).exit_code == 0)
        if $auto_merged { break }
    }
    let already_merged = $auto_merged
    if $already_merged {
        print 'run_workflow: auto-merge already landed — fast-forward pull'
        ok (do { git pull --ff-only origin $branch } | complete) 'git pull'
    } else {
        print $"run_workflow: squash-merging ($run_branch) into ($branch) \(provisional auto-merge, fabro-ab2c)"
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
                let landed = ((do {
                    git diff --quiet $run_branch $"origin/($branch)"
                } | complete).exit_code == 0)
                if $landed {
                    print 'run_workflow: auto-merge landed during squash fallback — syncing to origin, continuing'
                    ok (do { git reset --hard $"origin/($branch)" } | complete) 'git reset'
                } else {
                    let detail = ($pushed.stderr | str trim | default $pushed.stdout | str trim)
                    fail $"git push failed: ($detail)"
                }
            }
        }
    }

    print $"run_workflow: done — run ($run_id) integrated"
}
