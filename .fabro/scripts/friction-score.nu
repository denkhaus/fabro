#!/usr/bin/env nu
# friction-score.nu — deterministic 0.0–1.0 systemic-friction score for the
# dev loop, computed from tracker and journal (run) metrics. Consumed as a
# trigger condition by the architect workflow (and readable by
# humans/line-watch). No LLM judgment anywhere.
#
# Usage:
#   nu .fabro/scripts/friction-score.nu [--days 7] [--threshold 0.6]
#
# Data layer (cutover stage 5, denkhaus/work#988): the work tracker on
# denkhaus/work. The facade cannot enumerate >100 items (single page, no
# cursor), so the ITEM census reads the tracker through gh (host,
# read-only, paginated — the established audit posture; gh never writes).
# The claim ledger comes through the facade (`fabro work claims`) because
# that view IS small and the server owns it.
#
# Timestamp semantics: items carrying a `seeds:` label are MIGRATED —
# their GitHub timestamps are import artifacts (all 2026-10-25/26), so
# their true times come from the body trailers (Original-Created /
# Original-Closed). Native items use their GitHub timestamps. Window
# metrics (D, S) count NATIVE events only; the backlog age (B) uses true
# ages for every open item.
#
# Components (each 0.0–1.0, window = --days, default 7):
#   D drain-deficit   = 1 - min(1, native_closes / max(1, native_creates))
#   B backlog-stag    = min(1, median_open_age_days / 14)   [open develop queue]
#   S starvation      = 1 - min(1, native_closes / max(1, develop_runs))
#   P stuck-claims    = live claims granted >2 days ago / open items
#   score = 0.35·D + 0.25·B + 0.20·S + 0.20·P
#   verdict: <0.30 normal | 0.30–0.60 grind (watch) | >=0.60 architecture-due
#
# Output: one JSON object (score, verdict, components, window, computed_at).
# Exit 0 always (a score is a measurement, not a pass/fail).

const SERVER = 'https://mirtuell.net'
const PROJECT = 'fabro'
const TRACKER = 'denkhaus/work'

def iso-age-days [iso: string]: nothing -> float {
  let dt = ($iso | into datetime)
  ((date now) - $dt) / 1day
}

def cap1 [x]: nothing -> float {
  if $x > 1.0 { 1.0 } else { $x }
}

# One migrated item's original timestamp from its trailer block, falling
# back to the GitHub timestamp when the trailer is absent.
def true-time [item: record, github_field: string, trailer: string]: nothing -> string {
  # concat, NOT $"..." interpolation: the regex's (?<ts>...) group would
  # evaluate as a subexpression inside an interpolated string (the
  # lint-nu interpolated-regex rule — this hit its author once already)
  let pattern = ($trailer ++ ': (?<ts>[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z)')
  let from_trailer = ($item.body? | default '' | parse --regex $pattern | get -o ts | first | default '')
  if not ($from_trailer | is-empty) { $from_trailer } else { ($item | get -o $github_field | default '') }
}

def is-migrated [item: record]: nothing -> bool {
  ($item.labels? | default [] | any {|l| ($l | str starts-with 'seeds:')})
}

# The full project census: every issue carrying the project label, all
# states, paginated. ~10 pages for ~1000 items; deterministic order.
def tracker-census []: nothing -> list<record> {
  let run = (do { gh api --paginate $"repos/($TRACKER)/issues?state=all&labels=project%3A($PROJECT)&per_page=100" --jq '.[] | select(.pull_request == null) | {number, state, created_at, closed_at, labels: [.labels[].name], body}' } | complete)
  if $run.exit_code != 0 {
    print -e $"friction-score: tracker census failed: ($run.stderr | str trim | str substring 0..200)"
    exit 1
  }
  try { $run.stdout | lines | each {|l| $l | from json } } catch { [] }
}

def main [--days: int = 7, --threshold: float = 0.6]: nothing -> nothing {
  let census = (tracker-census)
  let open_items = ($census | where {|i| $i.state == 'open'})
  let window_start = ((date now) - ($days * 24hr))

  # Native tracker events in window (migrated items' GitHub timestamps are
  # import artifacts — they are historical traffic, not this window's).
  let native = ($census | where {|i| not (is-migrated $i)})
  let creates_in_w = ($native | where {|i|
    let ts = ($i.created_at? | default '')
    (not ($ts | is-empty)) and (($ts | into datetime) > $window_start) } | length)
  let closes_in_w = ($native | where {|i|
    let ts = ($i.closed_at? | default '')
    (not ($ts | is-empty)) and (($ts | into datetime) > $window_start) } | length)

  # D: drain deficit
  let denom = (if $creates_in_w > 0 { $creates_in_w } else { 1 })
  let drain = (($closes_in_w | into float) / ($denom | into float))
  let d = (1.0 - (cap1 $drain))

  # B: backlog stagnation — median true age of the OPEN develop queue
  # (lane:develop label; migrated ages come from Original-Created)
  let develop_open = ($open_items | where {|i| 'lane:develop' in ($i.labels? | default [])})
  let ages = ($develop_open | each {|i|
    let ts = (if (is-migrated $i) { true-time $i 'created_at' 'Original-Created' } else { ($i.created_at? | default '') })
    if ($ts | is-empty) { null } else { iso-age-days $ts }
  } | compact | sort)
  let median_age = (if ($ages | is-empty) { 0.0 } else {
    let n = ($ages | length)
    ($ages | get ((($n - 1) / 2) | math floor))
  })
  let b = (cap1 ($median_age / 14.0))

  # develop runs in window: develop-signature journals (a planner node
  # record) whose start ts falls inside the window; revisor/conductor
  # journals lack the planner signature
  let journal_files = (glob .fabro/journal/*.jsonl)
  mut develop_runs = 0
  for jf in $journal_files {
    let raw = (open --raw $jf | lines | compact)
    if ($raw | is-empty) { continue }
    if not ($raw | any {|l| ($l | str contains '"node":"planner"') }) { continue }
    let ts = (try { ($raw | first | from json | get ts? | default '') } catch { '' })
    if ($ts | is-empty) { continue }
    let ok = (try { ($ts | into datetime) > $window_start } catch { false })
    if $ok {
      $develop_runs = ($develop_runs + 1)
    }
  }

  # S: starvation — closes per develop run (1 item per run is the cadence)
  let rdenom = (if $develop_runs > 0 { $develop_runs } else { 1 })
  let close_rate = (($closes_in_w | into float) / ($rdenom | into float))
  let s = (1.0 - (cap1 $close_rate))

  # P: stuck claims — live ledger claims granted >2 days ago, over the
  # open backlog (the fabro-3c9d class; claims are the new in_progress)
  let claims_run = (do { ^fabro work --server $SERVER claims --project $PROJECT --json } | complete)
  let claims = (if $claims_run.exit_code == 0 {
    try { $claims_run.stdout | from json } catch { [] }
  } else { [] })
  let stuck = ($claims | where {|c|
    let ts = ($c.grantedAt? | default '')
    (not ($ts | is-empty)) and ((iso-age-days $ts) > 2.0) } | length)
  let open_n = ($open_items | length)
  let p = (if $open_n == 0 { 0.0 } else { cap1 (($stuck | into float) / ($open_n | into float)) })

  let score = ((0.35 * $d) + (0.25 * $b) + (0.20 * $s) + (0.20 * $p))
  let verdict = (if $score >= $threshold {
    'architecture-due'
  } else if $score >= 0.30 {
    'grind'
  } else {
    'normal'
  })

  {score: ($score | math round --precision 3),
   verdict: $verdict,
   threshold: $threshold,
   components: {
      drain_deficit: ($d | math round --precision 3),
      backlog_stagnation: ($b | math round --precision 3),
      starvation: ($s | math round --precision 3),
      stuck_claims: ($p | math round --precision 3)},
   inputs: {
      creates_in_window: $creates_in_w,
      closes_in_window: $closes_in_w,
      develop_runs_in_window: $develop_runs,
      median_open_age_days: ($median_age | math round --precision 1),
      stuck_live_claims: $stuck,
      open_total: $open_n,
      open_develop: ($develop_open | length)},
   window_days: $days,
   computed_at: (date now | date to-timezone utc | format date "%Y-%m-%dT%H:%M:%SZ")} | to json --raw | print
}
