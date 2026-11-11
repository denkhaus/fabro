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
# Parse-time self path: `path self` errors at run time since nu 0.115
# ("can't run after parse-time"), which broke the flock re-exec entry.
const SELF = (path self)

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

# Bench configuration as code (retro 2026-11-11, sprint 84): the local
# toolchain catalog entry must be INERT (docker: null) like production's
# — a stale finished image makes every workflow-declared env build
# refuse compile ("set one", work#1252's first-drive block). Idempotent:
# nulls the image only when the entry carries one.
def reconcile-toolchain-catalog [] {
    let server = 'http://127.0.0.1:32276'
    let s = (open ~/.fabro/auth.json | get servers | get -o $server)
    let token = ($s | get -o token | default ($s | get -o access_token | default ''))
    let auth = {Authorization: $"Bearer ($token)"}
    let full = (http get --headers $auth $"($server)/api/v1/environments/toolchain" --full)
    if (($full | get body | get image | get -o docker | default null) == null) {
        print "up: toolchain catalog entry already inert"
        return
    }
    let headers = ($full | get headers | get response | default [])
    let etag = ($headers | where name == etag | get -o value.0 | default '')
    if ($etag | is-empty) {
        print -e "up: no ETag on the toolchain environment — leaving the catalog untouched"
        return
    }
    let body = ($full | get body | reject id revision | update image {|_e|
        {docker: null, dockerfile: null}
    } | to json)
    http put --headers ({Authorization: $"Bearer ($token)", If-Match: $etag}) -t 'application/json' $"($server)/api/v1/environments/toolchain" $body | ignore
    print "up: toolchain catalog entry nulled (the inert production shape)"
}

def main [--locked] {
    if not $locked {
        # Entry: acquire the pipeline-wide flock by re-execing this script
        # under it. Streams nothing until the run finishes (output is
        # replayed below), then propagates the inner exit code verbatim.
        mkdir tmp
        let res = try {
            (^flock -n -E 42 $LOCK_PATH nu $SELF --locked | complete)
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
    just compose-up
    just wait-healthy
    reconcile-toolchain-catalog
    just smoke
    just clean
}
