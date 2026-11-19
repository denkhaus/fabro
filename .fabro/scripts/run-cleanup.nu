#!/usr/bin/env nu
# Terminal cleanup for fabro run sandboxes (work#1478). The fabro server
# executes this script in a run's sandbox when the run turns terminal and
# the sandbox is still reachable — BEFORE the container is released — so
# the run's build artifacts stop costing their retention-window writable
# layer (overlayfs upper-layer semantics: deleting files the run created
# reclaims their disk at once). The workflow declares the invocation
# (`[workflow] terminal_cleanup` in workflow.toml); this script owns WHAT
# is deleted: a bounded, whitelisted sweep — deterministic, never
# agent-driven (ADR-0015: project-owned scripts).
#
# Whitelist (exactly three classes, nothing else):
#   1. `target/` at the workspace root and ONE level deep (a nested repo
#      checkout layout). Cargo path-deps build into the workspace root's
#      single target dir, so the vendored workspaces are covered by it.
#   2. `/opt/target` — the toolchain image's prebuilt target dir — only
#      when present and removable.
#   3. `/tmp/fabro-*` — the line's scratch leftovers.
#
# What is NEVER touched, by construction:
#   - `.git` and every dot-entry: the enumeration skips hidden entries,
#     and only directories literally named `target` are candidates.
#   - `.fabro/journal` (and all of `.fabro/`): hidden, never enumerated.
#   - `.seeds`-era paths: hidden, never enumerated.
#   - anything outside the workspace plus the two absolute exceptions:
#     no other path is ever named.
#
# Contract: idempotent; exit 0 on success; on partial failure print one
# `run-cleanup:` reason per failed path to stderr and exit non-zero (the
# caller only warns). Stdout: exactly one `run-cleanup:` line reporting
# the removed paths and — when df measured the workspace filesystem —
# the approximate freed size.

# The used 1K blocks of the filesystem holding `path`, or null when df
# cannot answer (the caller then reports no measurement).
def df-used-kb [path: string] {
  let probe = (do { ^df -k --output=used $path } | complete)
  if $probe.exit_code != 0 {
    return null
  }
  let reading = ($probe.stdout | lines | reverse | get 0? | default '' | str trim)
  if ($reading | is-empty) {
    return null
  }
  try {
    $reading | into int
  } catch {
    null
  }
}

# The workspace's `target/` directories: the root's own and those of its
# visible (non-hidden) immediate child directories — one level deep, no
# deeper. Symlinked `target` entries are skipped: only real directories
# are candidates.
def collect-workspace-targets [workspace: string] {
  mut found = []
  let direct = ($workspace | path join 'target')
  if ($direct | path exists) and (($direct | path type) == 'dir') {
    $found = ($found | append $direct)
  }
  let children = (ls $workspace | where type == 'dir' | get name)
  for child in $children {
    let nested = ($child | path join 'target')
    if ($nested | path exists) and (($nested | path type) == 'dir') {
      $found = ($found | append $nested)
    }
  }
  $found
}

# Remove one whitelisted path, reporting a failure reason on stderr and
# returning 1 when the removal is refused (permissions, a racing writer);
# null on success.
def remove-path [path: string] {
  try {
    rm --recursive --force $path
    null
  } catch { |err|
    print -e $"run-cleanup: failed to remove ($path): ($err.msg)"
    1
  }
}

def main [] {
  let workspace = (pwd)
  let used_before = (df-used-kb $workspace)

  let targets = (try {
    collect-workspace-targets $workspace
  } catch { |err|
    print -e $"run-cleanup: could not enumerate the workspace: ($err.msg)"
    exit 1
  })

  # Class 1: the workspace's target dirs (root + one level deep).
  mut failures = 0
  mut removed = 0
  for target in $targets {
    let failure = (remove-path $target)
    if $failure == null {
      $removed = ($removed + 1)
    } else {
      $failures = ($failures + $failure)
    }
  }

  # Class 2: the toolchain image's prebuilt /opt/target, when present.
  let opt_target = '/opt/target'
  if ($opt_target | path exists) and (($opt_target | path type) == 'dir') {
    let failure = (remove-path $opt_target)
    if $failure == null {
      $removed = ($removed + 1)
    } else {
      $failures = ($failures + $failure)
    }
  }

  # Class 3: the line's /tmp scratch leftovers, files and directories.
  for leftover in (glob '/tmp/fabro-*') {
    let failure = (remove-path $leftover)
    if $failure == null {
      $removed = ($removed + 1)
    } else {
      $failures = ($failures + $failure)
    }
  }

  if $failures > 0 {
    exit 1
  }

  # One stdout line. The freed size is approximate: df reads the
  # filesystem the workspace sits on, and /opt/target may live on
  # another one.
  let used_after = (df-used-kb $workspace)
  if $used_before != null and $used_after != null {
    let freed = ($used_before - $used_after)
    if $freed > 0 {
      print $"run-cleanup: removed ($removed) path\(s\), freed approximately ($freed) KiB \(workspace filesystem\)"
      return
    }
  }
  print $"run-cleanup: removed ($removed) path\(s\)"
}
