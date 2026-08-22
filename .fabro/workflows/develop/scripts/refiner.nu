#!/usr/bin/env nu
# Deterministic painpoint delivery (ADR-0002): lift staged painpoints from
# .fabro/run-painpoints.jsonl into the platform mailbox (painpoints.jsonl on
# the platform meta branch). No LLM — pure transport.
#
# The platform branch is discovered generically: the single origin head
# matching refs/heads/meta/*. Ambiguity or absence skips delivery loudly
# (exit 0; staged file stays for the next run) so this step never fails the
# goal gate.

const staged = ".fabro/run-painpoints.jsonl"

if not ($staged | path exists) {
    print "no staged painpoints — nothing to deliver"
    exit 0
}
let raw = (open $staged | lines | compact)
if ($raw | is-empty) {
    print "staged file empty — nothing to deliver"
    rm $staged
    exit 0
}

let metas = (git ls-remote --heads origin "refs/heads/meta/*" | lines | parse --regex 'refs/heads/meta/(?P<branch>\S+)' | get branch | compact)
if ($metas | is-empty) {
    print "SKIP: no meta/* branch on origin — painpoints stay staged at $staged"
    exit 0
}
if ($metas | length) > 1 {
    print $"SKIP: ambiguous meta branches \(($metas | str join ', ')\) — painpoints stay staged at $staged"
    exit 0
}
let meta_branch = $"meta/($metas | get 0)"

let stamp = (date now | format date "%Y-%m-%dT%H:%M:%SZ")
let run = (git branch --show-current | str trim | parse --regex 'fabro/run/(?P<id>.+)$' | get -o id.0 | default "unknown-run")

# worktree on the platform branch, append, commit, push, clean up
let wt = (mktemp -d)
git fetch origin $meta_branch
git worktree add $wt $"origin/($meta_branch)" --detach
let mailbox = $"/($wt)/painpoints.jsonl"
let existing = if ($mailbox | path exists) { open $mailbox | lines | compact } else { [] }
let delivered = ($raw | each {|line|
    $"($line | str trim) \(delivered_at: ($stamp), run: ($run))"
})
($existing | append $delivered | str join "\n" | $"($in)\n") | save --force $mailbox

let n = ($delivered | length)
let msg = $"refiner: deliver ($n) painpoints from run ($run)"
let refspec = $"HEAD:refs/heads/($meta_branch)"
git -C $wt add painpoints.jsonl
git -C $wt commit -m $msg
git -C $wt push origin $refspec
git worktree remove --force $wt
rm $staged
let dn = ($delivered | length)
print $"delivered ($dn) painpoints to ($meta_branch)"
