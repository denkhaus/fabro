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
def deliver [raw: list<string>, meta_branch: string]: nothing -> nothing {
    let run = (current-run-id)
    let delivered = ($raw | each {|line|
        $"($line | str trim) \(delivered_at: (stamp), run: ($run))"
    })
    let n = ($delivered | length)

    let wt = (mktemp --directory)
    git fetch origin $meta_branch
    git worktree add $wt $"origin/($meta_branch)" --detach
    let mailbox = ($wt | path join $MAILBOX)
    let existing = (if ($mailbox | path exists) { open $mailbox | lines | compact } else { [] })
    ($existing | append $delivered | str join "\n" | $"($in)\n") | save --force $mailbox

    git -C $wt add $MAILBOX
    git -C $wt commit -m $"refiner: deliver ($n) painpoints from run ($run)"
    git -C $wt push origin $"HEAD:refs/heads/($meta_branch)"
    git worktree remove --force $wt
    rm $STAGED
    print $"delivered ($n) painpoints to ($meta_branch)"
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
