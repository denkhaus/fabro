# Full `just up` pipeline body (fabro-828c): the cross-process lock,
# recipe sequencing, and ALARM block. The justfile `up` recipe is a thin
# wrapper over this script (thin-recipe pattern; language axis = nu).
#
# LOCK (fabro-332e, partial): two overlapping `just up` runs raced the SPA
# dist mirror on 2026-08-25 and shipped an instance whose UI 404'd every
# asset while health stayed green. The lock file (tmp/just-up.lock) is
# held by flock for the WHOLE pipeline: this script re-execs itself under
# `flock -n -E 99`, so a second run aborts immediately with the holder's
# pid instead of corrupting the mirror.
#
# Pipeline: clean -> build-image -> install-cli -> run-images ->
# compose-up -> wait-healthy -> smoke -> clean (see the justfile header
# for the caching story — every slow part survives repeated runs).

const LOCK_FILE = 'tmp/just-up.lock'
const LOCK_PID_FILE = 'tmp/just-up.lock.pid'

# Terminal failure: loud ALARM block on stderr (same shape as the bash
# body this script replaced), then exit 1.
def alarm [holder: string] {
    let box_top = "╔══ ALARM: another 'just up' is already running (" + $holder + ") ══╗"
    print -e ''
    print -e $box_top
    print -e '║ A second pipeline would race bun build + the SPA dist mirror ║'
    print -e '║ and can ship an instance with a dead UI (2026-08-25 incident).║'
    print -e ('║ Wait for the running one, or kill ' + $holder + ' first.             ║')
    print -e '╚══════════════════════════════════════════════════════════════╝'
}

# The locked pipeline: run under flock by main. Records the holder pid
# for the ALARM message, then sequences the just recipes; a failing step
# propagates its exit code (nothing after it runs).
def run-locked [] {
    ($nu.pid | into string) | save --force $LOCK_PID_FILE
    for step in [clean build-image install-cli run-images compose-up wait-healthy smoke clean] {
        ^just $step
        let code = $env.LAST_EXIT_CODE
        if $code != 0 {
            exit $code
        }
    }
}

def main [--locked] {
    if $locked {
        run-locked
        return
    }
    mkdir tmp
    # Re-exec under flock: -E 99 marks lock contention (flock's own exit
    # codes otherwise collide with pipeline failure codes). `complete`
    # captures the exit code — a bare external would abort the script.
    let run = (do { ^flock -n -E 99 $LOCK_FILE nu $env.CURRENT_FILE --locked } | complete)
    let code = $run.exit_code
    if $code == 99 {
        let holder = (try { open --raw $LOCK_PID_FILE | str trim } catch { 'unknown pid' })
        alarm $holder
        exit 1
    }
    exit $code
}
