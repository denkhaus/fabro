#!/usr/bin/env nu
# Salvage-sweep smoke (fabro-f312, live shapes work#1477): the dump analysis
# is the sweep's decision core, so the battery pins it against fixture dump
# directories — the verdict-relevant shapes plus the affected-item
# extraction. The filing itself stays dry (the sweep's --dry-run arm covers
# it live).
#
# prompt-lint: synthetic-seed-ids
# This battery's `fabro-xxxx` literals are FIXTURE ids that must not exist
# in the tracker (fabro-cb5c): a verdict is exercised over plantable record
# bodies, so a real id would make the fixture depend on live tracker state.
# The marker suspends ONLY prompt-lint's resolvable-id check for this file.

const ANALYSIS = "salvage-analysis.nu"
source $ANALYSIS

# Build one fixture dump directory: run status kind, per-stage (outcome,
# optional patch body), optional stream export (events.jsonl content).
def fixture [kind: string, goal: string, stages: list, --events: string = ""]: nothing -> string {
  let dir = (mktemp -d)
  mkdir ($dir | path join 'stages')
  {
    spec: {settings: {run: {goal: {type: 'inline', value: $goal}}}},
    status: {kind: $kind},
  } | save ($dir | path join 'run.json')
  for stage in $stages {
    let sid = ($stage | get 0)
    mkdir ($dir | path join 'stages' $sid)
    {outcome: ($stage | get 1), notes: null, failure_reason: null, timestamp: '2026-11-19T00:00:00Z'}
      | save ($dir | path join 'stages' $sid 'status.json')
    if ($stage | get 2? | default '' | str length) > 0 {
      ($stage | get 2) | save --raw ($dir | path join 'stages' $sid 'diff.patch')
    }
  }
  if ($events | str length) > 0 {
    $events | save --raw ($dir | path join 'events.jsonl')
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

# One stream-export line for a tool call (the events.jsonl shape: a stream
# item whose progress record names the called tool).
def tool-event [tool: string]: nothing -> string {
  $"{\"run_id\":\"01FIXTURE0000000000000000AA\",\"stream_seq\":12,\"kind\":\"petri\",\"id\":\"step/3/2\",\"recorded_at\":\"2026-11-19T00:00:01Z\",\"event\":{\"name\":\"step.progress.recorded\",\"data\":{\"tool\":{\"name\":\"($tool)\"}}}}"
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
  if not ($ra.candidate and $ra.noteworthy and $ra.affected_item == 'fabro-a1b2') {
    print $"FAIL: failed+work should be a noteworthy candidate naming the item: ($ra | to json)"
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
  let d = (fixture 'failed' 'no item here' [['001-implementer@1' 'failed' '']])
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

  # F: a failed run whose stream records a work-item WRITE tool call — the
  # filing it made never left its unpublished sandbox (work#1477: the live
  # stranding class since the tracker cutover, denkhaus/work#988/#990;
  # red-first proof 2026-11-19: the retired store-hunk counter returned
  # noteworthy=false for exactly this fixture).
  let f = (fixture 'failed' 'file the follow-up' [['001-file@1' 'failed' $journal_patch]] --events (tool-event 'fabro_work_create'))
  let rf = (analyze-dump $f)
  if not ($rf.candidate and $rf.noteworthy and $rf.work_filings == 1) {
    print $"FAIL: a recorded work-item write call is stranded work: ($rf | to json)"
    exit 1
  }

  # F2: the other write surfaces count too — a closure call on the record.
  let f2 = (fixture 'failed' 'close the item' [['001-close@1' 'failed' '']] --events (tool-event 'fabro_work_close'))
  let rf2 = (analyze-dump $f2)
  if not ($rf2.candidate and $rf2.noteworthy and $rf2.work_filings == 1) {
    print $"FAIL: a recorded work-item close call is stranded work: ($rf2 | to json)"
    exit 1
  }

  # G: the noise guard — the READ/claim tools every develop run calls
  # (`fabro_work_next`, `fabro_work_show`) never count: counting them would
  # make every terminal run noteworthy and turn the sweep into noise.
  let g = (fixture 'failed' 'claim and die' [['001-file@1' 'failed' '']] --events ((tool-event 'fabro_work_next') + (char nl) + (tool-event 'fabro_work_show')))
  let rg = (analyze-dump $g)
  if not ($rg.candidate and not $rg.noteworthy and $rg.work_filings == 0) {
    print $"FAIL: the claim/read tools are not stranded work: ($rg | to json)"
    exit 1
  }

  print 'salvage-sweep-smoke: OK — candidate/noteworthy/green-lie/bookkeeping/work-filings/close-call/read-tool-noise/empty/clean verdicts pinned'

}
