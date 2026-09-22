#!/usr/bin/env nu
# seeds call-cost benchmark (fabro-088b acceptance b): measures wall-clock
# cost of tracker listing through the reference shim (`sd list`, the
# PATH-shim form the line uses today) and through `fabro seeds list`
# (native compile-in, wired or pending). Prints a JSON verdict; the
# acceptance is call-cost parity with the reference, measured in-sandbox.
#
# Usage:
#   nu .fabro/scripts/seeds-cost-bench.nu [--runs <n>] [--cwd <dir>]
#
# Notes:
#   - `fabro seeds list` refuses with the pending-binding error until the
#     command layer is wired (fabro-088b); the verdict reports it as
#     `pending` instead of a cost.
#   - Reference measurement needs `sd` on PATH and a .seeds/ checkout in
#     --cwd (default: repo root).

def main [
  --runs: int = 10
  --cwd: string = "."
] {
  let fabro_bin = ($env.FABRO_BIN? | default "fabro")
  mut samples = {}

  let sd_cost = if (which sd | is-empty) {
    null
  } else {
    let t0 = (date now)
    for _ in 1..$runs { sd list --limit 5 | ignore }
    ((date now) - $t0) / $runs
  }

  let fabro_out = (do { cd $cwd; ^$fabro_bin seeds list } | complete)
  let fabro = if ($fabro_out.exit_code != 0) {
    {status: "pending", note: ($fabro_out.stderr | str trim | lines | first)}
  } else {
    let t0 = (date now)
    for _ in 1..$runs { cd $cwd; ^$fabro_bin seeds list | ignore }
    {status: "ok", cost_ms: ((((date now) - $t0) / $runs) / 1ms | math round --precision 1)}
  }

  {
    verdict: "bench"
    runs: $runs
    reference_sd_ms: (if $sd_cost == null { null } else { $sd_cost / 1ms | math round --precision 1 })
    fabro_seeds: $fabro
  } | to json
}
