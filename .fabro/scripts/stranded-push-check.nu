#!/usr/bin/env nu
# Stranded-push check (denkhaus/work#1358, the 2026-11-15 fork.37
# near-deploy): a branch-door PR that MERGED takes no further pushes —
# commits pushed to such a branch afterwards strand silently outside
# origin/denkhaus while every PR-level view reads "merged". This check
# verifies COMMITS, not PR state: for every recently merged PR whose
# head branch still exists on origin, the branch tip must be an
# ancestor of origin/denkhaus, and the PR's merge commit must sit in
# the trunk. Deterministic script first: the line-watch procedure keeps
# only the call + verdict routing.
#
# Verdicts (one line per branch):
#   clean:<branch> tip in trunk | clean:<branch> deleted post-merge |
#   stranded:<branch> tip holds commits outside origin/denkhaus |
#   stranded:<branch> merge commit missing from trunk |
#   check:skipped:<reason>
# Exit 0 when nothing stranded, 1 otherwise.

def is-ancestor [tip: string, trunk: string] {
  let probe = (do { ^git merge-base --is-ancestor $tip $trunk } | complete)
  $probe.exit_code == 0
}

# One verdict line for a branch tip against the trunk ref.
def verdict-line [branch: string, tip: string, trunk: string] {
  if (is-ancestor $tip $trunk) {
    print $"clean:($branch) tip in trunk"
  } else {
    print $"stranded:($branch) tip holds commits outside ($trunk)"
  }
}

def main [
  --repo: string = 'denkhaus/fabro'      # the forge repo the PRs live in
  --limit: int = 30                      # how many merged PRs to inspect, newest first
  --trunk: string = 'origin/denkhaus'    # the trunk ref commits must be ancestors of
  --branch: string = ''                  # direct mode: check exactly this branch tip...
  --tip: string = ''                     # ...at this sha (the negative dry-run door)
] {
  let ensure_fetch = (do { ^git fetch origin denkhaus --quiet } | complete)
  if $ensure_fetch.exit_code != 0 {
    print -e $"stranded-push-check: git fetch failed — ($ensure_fetch.stderr | str substring 0..160)"
    exit 1
  }

  # Direct mode: one raw assertion pair, no gh — the negative dry-run door.
  if $branch != '' {
    if $tip == '' {
      print -e 'stranded-push-check: --branch requires --tip'
      exit 1
    }
    verdict-line $branch $tip $trunk
    exit (if (is-ancestor $tip $trunk) { 0 } else { 1 })
  }

  let gh_argv = ['pr' 'list' '--repo' $repo '--state' 'merged' '--limit' ($limit | into string) '--json' 'number,headRefName,headRefOid,mergeCommit,mergedAt']
  let prs = (do { ^gh ...$gh_argv } | complete)
  if $prs.exit_code != 0 {
    print -e $"stranded-push-check: gh pr list failed — ($prs.stderr | str substring 0..160)"
    exit 1
  }
  let merged = ($prs.stdout | from json)
  if ($merged | is-empty) {
    print 'check:skipped no merged PRs in range'
    exit 0
  }

  mut stranded = 0
  for pr in $merged {
    let branch = ($pr.headRefName? | default '')
    if $branch == '' {
      print 'check:skipped pr without a head branch'
      continue
    }
    let ls_argv = ['ls-remote' 'origin' $"refs/heads/($branch)"]
    let remote = (do { ^git ...$ls_argv } | complete)
    if $remote.exit_code != 0 or ($remote.stdout | str trim) == '' {
      print $"clean:($branch) deleted post-merge"
      continue
    }
    let tip = ($remote.stdout | str trim | split column ' ' ref sha | get 0.sha)
    let merge_oid = ($pr.mergeCommit?.oid? | default '')
    if $merge_oid != '' and (not (is-ancestor $merge_oid $trunk)) {
      print $"stranded:($branch) merge commit missing from trunk"
      $stranded += 1
      continue
    }
    verdict-line $branch $tip $trunk
    if (not (is-ancestor $tip $trunk)) {
      $stranded += 1
    }
  }
  exit (if $stranded > 0 { 1 } else { 0 })
}
