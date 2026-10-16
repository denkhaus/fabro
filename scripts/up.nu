#!/usr/bin/env nu
# Full local pipeline (thin target of `just up`): clean, build image,
# install CLI, run-images, compose up, wait for health, smoke — then a
# final clean. Sequencing-only: every step delegates to its own just
# recipe / script.
#
# LOCK (fabro-332e, partial): two overlapping `just up` runs raced the SPA
# dist mirror on 2026-08-25 and shipped an instance whose UI 404'd every
# asset while health stayed green. The lock (tmp/just-up.lock, held by
# ^flock for the WHOLE pipeline) makes the second run abort immediately
# with the holder's pid instead of corrupting the mirror. flock stays an
# external (sandbox-without-nu failure mode does not apply here — this
# script only runs on the host): the entry invocation re-execs this same
# script under `flock -n`, so the lock covers every step below. Exit code
# 42 is flock's distinctive "lock busy" code (-E 42).

const LOCK_PATH = "tmp/just-up.lock"

# Print the cross-process ALARM block and exit 1 (second-run refusal).
def alarm [holder: string] {
    let h = ($holder | str trim)
    print -e ""
    print -e "╔══ ALARM: another 'just up' is already running ($h) ══╗"
    print -e "║ A second pipeline would race bun build + the SPA dist mirror ║"
    print -e "║ and can ship an instance with a dead UI (2026-08-25 incident).║"
    print -e "║ Wait for the running one, or kill ($h) first.             ║"
    print -e "╚══════════════════════════════════════════════════════════════╝"
    exit 1
}

def main [--locked] {
    if not $locked {
        # Entry: acquire the pipeline-wide flock by re-execing this script
        # under it. Streams nothing until the run finishes (output is
        # replayed below), then propagates the inner exit code verbatim.
        mkdir tmp
        let res = try {
            (^flock -n -E 42 $LOCK_PATH nu (path self) --locked | complete)
        } catch {|err|
            { stdout: "", stderr: $"up: unable to run flock: ($err.msg)", exit_code: 127 }
        }
        if $res.exit_code == 42 {
            let holder = (do { open tmp/just-up.lock.pid } | complete | get stdout | default "unknown pid")
            alarm $holder
        }
        if ($res.stdout | str trim) != "" { print ($res.stdout | str trim -r) }
        if ($res.stderr | str trim) != "" { print -e ($res.stderr | str trim -r) }
        exit $res.exit_code
    }
    # Locked branch: the flock is held for the whole pipeline.
    $nu.pid | save --force tmp/just-up.lock.pid
    just clean
    just build-image
    just install-cli
    just run-images
    just compose-up
    just wait-healthy
    just smoke
    just clean
}
