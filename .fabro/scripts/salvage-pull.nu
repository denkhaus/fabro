#!/usr/bin/env nu
# salvage-pull.nu — pull a terminal run's stranded work out of its sandbox as a git patch.
#
# STATUS (2026-11-17): mechanics proven (inspect → .git copy → overlay → patch),
# but UNUSABLE IN PRACTICE until the sandbox copy throughput is fixed:
#   BLOCKED BY denkhaus/work#1416 — recursive `sandbox cp` is per-file round
#   trips; a 166 MB .git ran ~3 min and a ~30 MB source tree exceeded 4 min
#   (three aborted drives). Once #1416 lands (bulk archive path + progress),
#   this script is the executor with NO further changes expected.
# Serves denkhaus/work#1415 (the salvage pointer for run 01M7MWB3 / work#1159's
# stranded supplement arm — recoverable until sandbox retention reaps it,
# ~2026-11-24). Verdict vocabulary pairs with salvage-sweep.nu (which DECIDES;
# this EXECUTES).
#
# WHY: a failed run's implementer work usually sits UNCOMMITTED in the sandbox
# worktree (checkpoint commits only land at stage boundaries), and the run
# branch is never pushed to origin. The sandbox dies with the 7-day retention
# window — this script is the bounded window in which salvage is possible.
#
# WHAT IT DOES (all mechanical, no LLM):
#   1. inspects the run (refuses non-terminal runs), reads base + final shas
#   2. copies the sandbox `.git` (full history incl. the run's checkpoint commits)
#   3. copies the requested workspace paths (the UNCOMMITTED working tree)
#   4. rebuilds the final-checkpoint tree locally, overlays the copied files,
#      and writes the delta as a single `salvage.patch`
#
# OUTPUT: <dest>/salvage.patch + a diffstat. Apply recipe (printed at the end):
#   git checkout -b fix/salvage-<run> origin/denkhaus && git apply -3 salvage.patch
#   then verify with real commands and land via the branch door citing the run.
#
# NOTE: the CLI exposes no sandbox ls/search door (work#1416's third demand),
# so the copy set is declared, not discovered. The default covers the Rust
# workspace source set and excludes apps/ (node_modules weight) and vendor/
# (vendored edits are session work, not run work). Narrow --paths when the
# touched surface is known — it is faster.
#
# Usage:
#   nu .fabro/scripts/salvage-pull.nu <run-id> [--dest <dir>] [--paths <csv>]
#       [--fabro ./target/debug/fabro] [--server https://mirtuell.net]

# The one terminal vocabulary (work#1446): the shared predicate —
# succeeded/failed/human_needed/dead, both canceled spellings accepted.
# A parked human_needed run is TERMINAL (GLOSSARY, work#1072): its
# stranded work is as salvageable as a failed run's.
use lib/terminal_status.nu [is-terminal-kind]

# Copy one path out of the run sandbox; missing remote paths are a soft skip.
def sandbox-copy [fabro: string, server: string, run: string, remote: string, into: string] {
    let dst = ($into | path join $remote)
    print $"salvage-pull: copying ($remote)…"  # work#1416: until the bulk path lands this can take minutes per path — the print is the progress bar
    let res = (run-external $fabro ...["sandbox" "cp" "--server" $server $"($run):/workspace/($remote)" $dst "-r"] | complete)
    if ($res.exit_code != 0) {
        let why = ($res.stderr | str trim | str replace -a "\n" " " | str substring 0..120)
        print $"salvage-pull: skip ($remote) — ($why)"
    }
}

def main [
    run_id: string,                # the terminal run to salvage
    --dest: string = "",           # output dir (default: /tmp/opencode/salvage-<run-id>)
    --paths: string = "lib,docs,scripts,.fabro,Cargo.toml,Cargo.lock,justfile",
    --fabro: string = "./target/debug/fabro", # the tree build carries sandbox cp
    --server: string = "https://mirtuell.net",
] {
    let dest = (if ($dest | is-empty) { $"/tmp/opencode/salvage-($run_id)" } else { $dest })
    if ($dest | path exists) {
        print $"salvage-pull: refusing — ($dest) already exists (remove it or pass --dest)"
        exit 1
    }
    mkdir $dest

    # 1. inspect: refuse non-terminal runs, read the sha pair
    let inspect = (run-external $fabro ...["inspect" $run_id "--server" $server "--json"] | complete)
    if ($inspect.exit_code != 0) {
        print $"salvage-pull: inspect failed — ($inspect.stderr | str trim)"
        exit 1
    }
    let run = ($inspect.stdout | from json | first)
    let status = ($run.status.kind? | default "unknown")
    if (not (is-terminal-kind $status)) {
        print $"salvage-pull: refusing — run ($run_id) is not terminal (status: ($status))"
        exit 1
    }
    let final = ($run.conclusion?.final_git_commit_sha? | default "")
    let base = ($run.start_record?.base_sha? | default "")
    if ($final | is-empty) {
        print "salvage-pull: refusing — no final checkpoint commit on the run branch"
        exit 1
    }
    print $"salvage-pull: run ($run_id) terminal \($status\), base ($base), final ($final)"

    # 2. the sandbox .git (full history + checkpoint commits)
    print "salvage-pull: copying the sandbox .git (this is the slow step)…"
    let gitres = (run-external $fabro ...["sandbox" "cp" "--server" $server $"($run_id):/workspace/.git" $"($dest)/git" "-r"] | complete)
    if ($gitres.exit_code != 0) {
        let why = ($gitres.stderr | str trim | str substring 0..200)
        print $"salvage-pull: .git copy failed — is the sandbox still alive? ($why)"
        exit 1
    }

    # 3. the declared worktree paths (the uncommitted side)
    print $"salvage-pull: copying worktree paths: ($paths)"
    mkdir $"($dest)/worktree"
    for p in ($paths | split row "," | str trim) {
        sandbox-copy $fabro $server $run_id $p $"($dest)/worktree"
    }

    # 4. rebuild the final-checkpoint tree and overlay the copies
    print "salvage-pull: rebuilding the checkpoint tree locally…"
    let cl = (run-external git ...["clone" "--quiet" "--bare" $"($dest)/git" $"($dest)/bare.git"] | complete)
    if ($cl.exit_code != 0) {
        print $"salvage-pull: bare clone failed — ($cl.stderr | str trim)"
        exit 1
    }
    let wt = $"($dest)/wt"
    let wtres = (run-external git ...["-C" $"($dest)/bare.git" "worktree" "add" "--quiet" "--detach" $wt $final] | complete)
    if ($wtres.exit_code != 0) {
        print $"salvage-pull: worktree add failed — ($wtres.stderr | str trim)"
        exit 1
    }
    let top = (ls -a $"($dest)/worktree" | where $it.name != $"($dest)/worktree" | get name)
    for it in $top {
        cp -r $it $wt
    }

    # 5. the patch: the overlay delta vs the final checkpoint
    run-external git ...["-C" $wt "add" "-A"]
    let stat = (run-external git ...["-C" $wt "diff" "--cached" "--stat"] | complete)
    let patchfile = $"($dest)/salvage.patch"
    run-external git ...["-C" $wt "diff" "--cached" "--binary"] | save -f $patchfile
    let size = (ls -l $patchfile | get 0.size | into int)
    if ($size == 0) {
        print "salvage-pull: the overlay delta is EMPTY — the sandbox worktree matches the final checkpoint (clean, or the copy set missed the touched paths: widen --paths)"
        exit 2
    }
    print $stat.stdout
    print $"salvage-pull: patch written — ($patchfile) \(($size) bytes\)"
    print $"salvage-pull: next: git checkout -b fix/salvage-($run_id) origin/denkhaus && git apply -3 ($patchfile) — verify with real commands, land via the branch door citing the run."
}
