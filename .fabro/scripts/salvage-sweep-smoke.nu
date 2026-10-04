#!/usr/bin/env nu
# Salvage-sweep smoke (fabro-f312): the dump analysis is the sweep's
# decision core, so the battery pins it against fixture dump directories —
# the four verdict-relevant shapes plus the affected-seed extraction. The
# filing itself stays dry (the sweep's --dry-run arm covers it live).

const ANALYSIS = "salvage-analysis.nu"
source $ANALYSIS

# Build one fixture dump directory: run status kind, per-stage (outcome,
# optional patch body), and the goal text.
def fixture [kind: string, goal: string, stages: list]: nothing -> string {
  let dir = (mktemp -d)
  mkdir ($dir | path join 'stages')
  {
    spec: {settings: {run: {goal: {type: 'inline', value: $goal}}}},
    status: {kind: $kind},
  } | save ($dir | path join 'run.json')
  for stage in $stages {
    let sid = ($stage | get 0)
    mkdir ($dir | path join 'stages' $sid)
    {outcome: ($stage | get 1), notes: null, failure_reason: null, timestamp: '2026-10-04T00:00:00Z'}
      | save ($dir | path join 'stages' $sid 'status.json')
    if ($stage | get 2? | default '' | str length) > 0 {
      ($stage | get 2) | save --raw ($dir | path join 'stages' $sid 'diff.patch')
    }
  }
  $dir
}

def real_patch [lines: int]: nothing -> string {
  let body = (1..$lines | each {|i| $"+line ($i)"} | str join (char nl))
  $"diff --git a/lib/x.rs b/lib/x.rs
new file mode 100644
--- a/lib/x.rs
+++ b/lib/x.rs
@@ -0,0 +1,($lines) @@
($body)
"
}

def main [] {
  let journal_patch = "diff --git a/.fabro/journal/run1.jsonl b/.fabro/journal/run1.jsonl
  new file mode 100644
  --- a/.fabro/journal/run1.jsonl
  +++ b/.fabro/journal/run1.jsonl
  @@ -0,0 +1 @@
  +x
  "

  # A: failed run with real stranded work -> candidate, noteworthy
  let a = (fixture 'failed' 'implement fabro-a1b2' [['001-implementer@1' 'failed' (real_patch 25)]])
  let ra = (analyze-dump $a)
  if not ($ra.candidate and $ra.noteworthy and $ra.affected_seed == 'fabro-a1b2') {
    print $"FAIL: failed+work should be a noteworthy candidate naming the seed: ($ra | to json)"
    exit 1
  }

  # B: failed run whose only diff is its own journal -> candidate, NOT noteworthy
  let b = (fixture 'failed' 'implement fabro-b2c3' [['001-implementer@1' 'failed' $journal_patch]])
  let rb = (analyze-dump $b)
  if not ($rb.candidate and not $rb.noteworthy) {
    print $"FAIL: journal-only work is bookkeeping: ($rb | to json)"
    exit 1
  }

  # C: green-lie — run succeeded while a stage failed, with real work -> candidate
  let c = (fixture 'succeeded' 'implement fabro-c4d5' [['001-implementer@1' 'failed' (real_patch 30)]])
  let rc = (analyze-dump $c)
  if not ($rc.candidate and $rc.green_lie and $rc.noteworthy) {
    print $"FAIL: a failed stage under a green run is a green-lie candidate: ($rc | to json)"
    exit 1
  }

  # D: no diff at all -> no crash, nothing noteworthy
  let d = (fixture 'failed' 'no seed here' [['001-implementer@1' 'failed' '']])
  let rd = (analyze-dump $d)
  if not ($rd.candidate and not $rd.noteworthy and $rd.changed == 0) {
    print $"FAIL: a diff-less run must not crash and is not noteworthy: ($rd | to json)"
    exit 1
  }

  # E: a clean succeeded run is never a candidate
  let e = (fixture 'succeeded' 'implement fabro-e6f7' [['001-implementer@1' 'succeeded' (real_patch 40)]])
  let re = (analyze-dump $e)
  if $re.candidate {
    print $"FAIL: a succeeded run with no failed stage is not a candidate: ($re | to json)"
    exit 1
  }

  # F: a failed run whose ONLY product is a tracker ADDITION — the seeds it
  # filed never landed (fabro-f312 closeout, 2026-10-04: a revisor whose file
  # stage died strands exactly this).
  let filed_patch = 'diff --git a/.seeds/issues.jsonl b/.seeds/issues.jsonl
--- a/.seeds/issues.jsonl
+++ b/.seeds/issues.jsonl
@@ -1,1 +1,2 @@
 {"id":"fabro-1111","title":"existing","status":"open"}
+{"id":"fabro-3333","title":"stranded filing","status":"open"}
'
  let f = (fixture 'failed' 'file the findings' [['001-file@1' 'failed' $filed_patch]])
  let rf = (analyze-dump $f)
  if not ($rf.candidate and $rf.noteworthy and $rf.added_seeds == 1) {
    print $"FAIL: added tracker records are stranded work: ($rf | to json)"
    exit 1
  }

  # G: the noise guard — a tracker REWRITE (a claim, a status change) is
  # churn every pass produces, never work.
  let claim_patch = 'diff --git a/.seeds/issues.jsonl b/.seeds/issues.jsonl
--- a/.seeds/issues.jsonl
+++ b/.seeds/issues.jsonl
@@ -1,1 +1,1 @@
-{"id":"fabro-1111","title":"existing","status":"open"}
+{"id":"fabro-1111","title":"existing","status":"in_progress"}
'
  let g = (fixture 'failed' 'claim and die' [['001-file@1' 'failed' $claim_patch]])
  let rg = (analyze-dump $g)
  if not ($rg.candidate and not $rg.noteworthy and $rg.added_seeds == 0) {
    print $"FAIL: a tracker rewrite is not stranded work: ($rg | to json)"
    exit 1
  }

  print 'salvage-sweep-smoke: OK — candidate/noteworthy/green-lie/bookkeeping/added-seeds/rewrite/empty/clean verdicts pinned'

}
