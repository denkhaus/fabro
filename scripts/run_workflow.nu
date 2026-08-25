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
#        - run branch already contained in origin/<branch> -> pull --ff-only
#          (auto-merge did its job)
#        - else squash-merge fabro/run/<id> into <branch>, push
#          (PROVISIONAL until auto-merge works, fabro-ab2c; the leftover
#          PR on GitHub can be closed as already-integrated)
#   6. fabro ask <id> --json with scripts/prompts/improve.md
#        -> answer saved to .fabro/reviews/<workflow>/<run-id>.md (with run header)
#   7. commit + push the review file
#
# Any failure prints an ALARM block and exits 1; nothing is merged and no
# review is committed for a red run.
#
# External calls follow the repo style (qualitygate.nu): literal external
# commands in `do { ^cmd ... } | complete`, exit_code checked. No generic
# arg-spread wrapper — rest params would swallow --flags meant for the
# external command.

const PROMPT_FILE = 'scripts/prompts/improve.md'
const REVIEWS_DIR = '.fabro/reviews'
const SERVER_DEFAULT = 'http://127.0.0.1:32276'

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
            do { ^$fabro_bin create $workflow --json --server $server } | complete
        } else {
            do { ^$fabro_bin create $workflow --goal $goal --json --server $server } | complete
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
    let already_merged = ((do {
        git merge-base --is-ancestor $run_branch $"origin/($branch)"
    } | complete).exit_code == 0)
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
            ok (do { git push origin $branch } | complete) 'git push'
        }
    }

    # ── 6. Ask-Fabro improve review ──────────────────────────────────
    if not ($PROMPT_FILE | path exists) {
        fail $"prompt file missing: ($PROMPT_FILE)"
    }
    let prompt = (open --raw $PROMPT_FILE)
    print 'run_workflow: asking Fabro for the improve review (this can take a while)'
    let asked = (do {
        ^$fabro_bin ask $run_id --prompt $prompt --json --server $server
    } | complete)
    ok $asked 'fabro ask'
    # ask --json emits JSONL events; the answer is the LAST turn.succeeded
    let events = (
        $asked.stdout
        | lines
        | compact
        | each {|line| try { $line | from json } catch { null } }
        | where $it != null
    )
    let turn = ($events | where event == 'run.session.turn.succeeded' | last)
    let review = ($turn | get -o properties | get -o output | default '' | into string)
    if ($review | str trim | is-empty) {
        fail $"ask produced no answer text (events seen: ($events | length))"
    }

    let review_dir = $"($REVIEWS_DIR)/($workflow)"
    mkdir $review_dir
    let wall_min = (($info | get -o timing | get -o wall_time_ms | default 0) / 60000 | math round --precision 1)
    let cost = (($info | get -o total_usd_micros | default 0) / 1_000_000)
    let generated = (date now | format date '%Y-%m-%d %H:%M%z')
    let header = (
        $"# Improve review — run ($run_id)\n\n"
        + $"- workflow: ($workflow)\n"
        + $"- branch integrated: ($branch)\n"
        + $"- status: ($status) \(($reason)), ($wall_min) min, \$($cost)\n"
        + $"- generated: ($generated) by `fabro ask` with `scripts/prompts/improve.md`\n\n"
        + "---\n\n"
    )
    let review_path = $"($review_dir)/($run_id).md"
    ($header + $review + "\n") | save --force $review_path
    print $"run_workflow: review saved ($review_path)"

    # ── 7. commit the review ─────────────────────────────────────────
    ok (do { git add $review_path } | complete) 'git add'
    let nothing_staged = ((do { git diff --cached --quiet } | complete).exit_code == 0)
    if $nothing_staged {
        print 'run_workflow: review unchanged — nothing to commit'
    } else {
        ok (do { git commit -m $"reviews: ($run_id)" } | complete) 'git commit'
        ok (do { git push origin $branch } | complete) 'git push'
    }

    print $"run_workflow: done — run ($run_id) integrated and reviewed"
}
