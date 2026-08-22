#!/usr/bin/env nu
# Deterministic painpoint delivery (ADR-0002): lift staged painpoints from
# .fabro/run-painpoints.jsonl into the platform mailbox (painpoints.jsonl on
# the platform meta branch). No LLM — pure transport.
#
# The platform branch is discovered generically: the single origin head
# matching refs/heads/meta/*. Ambiguity or absence skips delivery loudly
# (exit 0; staged file stays for the next run) so this step never fails the
# goal gate.

const STAGED = ".fabro/run-painpoints.jsonl"
const MAILBOX = "painpoints.jsonl"

# ---------------------------------------------------------------------------
# helpers
# ---------------------------------------------------------------------------

def current-run-id []: nothing -> string {
    git branch --show-current
    | str trim
    | parse --regex 'fabro/run/(?P<id>.+)$'
    | get -o id.0
    | default "unknown-run"
}

def staged-lines []: nothing -> list<string> {
    open $STAGED | lines | compact
}

# The single origin head under refs/heads/meta/* as a FULL branch name
# (meta/<name>), or null when absent or ambiguous — delivery then skips
# loudly instead of guessing. The prefix is part of the contract: callers
# use the result directly as a branch name.
def single-meta-branch []: nothing -> any {
    let metas = (
        git ls-remote --heads origin "refs/heads/meta/*"
        | lines
        | parse --regex 'refs/heads/meta/(?P<branch>\S+)'
        | get branch
        | compact
    )
    match ($metas | length) {
        1 => $"meta/($metas | get 0)"
        _ => null
    }
}

def stamp []: nothing -> string {
    date now | format date "%Y-%m-%dT%H:%M:%SZ"
}

# Append the delivered rows to the mailbox inside a detached worktree of
# the platform branch, commit, push, remove the worktree. Ownership is
# documented: a left-over worktree would poison the next run.
# Network-tolerant delivery (run-12 review): a fetch/push failure must
# never fail this node or destroy the staged file — the design intent is
# 'never fails the goal gate'. On any git error: clean up the worktree,
# keep the staged file, report loudly, exit 0 (next run retries).
def deliver [raw: list<string>, meta_branch: string]: nothing -> nothing {
    let run = (current-run-id)
    let delivered = ($raw | each {|line|
        $"($line | str trim) \(delivered_at: (stamp), run: ($run))"
    })
    let n = ($delivered | length)

    let wt = (mktemp --directory)
    let ok = (try {
        let fetch = (do { git fetch origin $meta_branch } | complete)
        if $fetch.exit_code != 0 { error make --unspanned {msg: $"fetch failed: ($fetch.stderr)"} }
        let add = (do { git worktree add $wt $"origin/($meta_branch)" --detach } | complete)
        if $add.exit_code != 0 { error make --unspanned {msg: $"worktree failed: ($add.stderr)"} }
        let mailbox = ($wt | path join $MAILBOX)
        let existing = (if ($mailbox | path exists) { open $mailbox | lines | compact } else { [] })
        ($existing | append $delivered | str join "\n" | $"($in)\n") | save --force $mailbox
        git -C $wt add $MAILBOX
        git -C $wt commit -m $"refiner: deliver ($n) painpoints from run ($run)"
        let push = (do { git -C $wt push origin $"HEAD:refs/heads/($meta_branch)" } | complete)
        if $push.exit_code != 0 { error make --unspanned {msg: $"push failed: ($push.stderr)"} }
        true
    } catch {|err|
        print $"SKIP: delivery error kept staged file: ($err.msg)"
        false
    })
    # worktree cleanup must not mask the result
    let _ = (do { git worktree remove --force $wt } | complete)
    if $ok {
        rm $STAGED
        print $"delivered ($n) painpoints to ($meta_branch)"
    } else {
        print $"painpoints stay staged at ($STAGED) for the next run"
    }
}

# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------

def main []: nothing -> nothing {
    if not ($STAGED | path exists) {
        print "no staged painpoints — nothing to deliver"
        return
    }
    let raw = (staged-lines)
    if ($raw | is-empty) {
        print "staged file empty — nothing to deliver"
        rm $STAGED
        return
    }

    let meta_branch = (single-meta-branch)
    match $meta_branch {
        null => {
            print $"SKIP: no single meta/* branch on origin — painpoints stay staged at ($STAGED)"
        }
        _ => { deliver $raw $meta_branch }
    }
}
