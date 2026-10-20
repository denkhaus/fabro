#!/usr/bin/env nu
# Per-criterion check transcript — the recording half of the evidence
# pipe. The IMPLEMENTER runs each acceptance-criterion check through
# this wrapper on implement runs; the PLANNER runs the per-criterion
# verification checks the same way on verification-only claims
# (fabro-f759 — a 1.7KB "(no seed-work files to diff)" capture once
# forced the reviewer to re-derive every criterion with tools). The
# wrapper records the command, its combined output, and its exit code
# (stamped with the recorder via --by) into a run-scoped transcript
# file that evidence.nu inlines as a `recorded checks` capture section.
# Motivation (fabro-d89a): an implementer's temp-fixture negative-path
# proof (fixtures built under `mktemp -d`, deleted afterwards) used to
# die with the fixtures, so the reviewer re-ran the entire proof
# itself; the transcript keeps the proof alive inside the capture.
#
# The transcript lives under /tmp, keyed by run id (parsed from the run
# branch `fabro/run/<id>` — the same parse evidence.nu performs, so both
# halves agree with zero wiring): OUTSIDE the worktree, so no
# checkpoint, no diff, no run-scope machinery ever sees it, while the
# same run sandbox's later evidence capture reads it back. Non-run
# branches degrade to the id "local" so manual invocations work.
#
# Usage (implementer):
#   nu .fabro/workflows/develop/scripts/check-transcript.nu -- '<check command>'
# Usage (planner, verification-only claims — fabro-f759):
#   nu .fabro/workflows/develop/scripts/check-transcript.nu --by planner -- '<check command>'
#   nu .fabro/workflows/develop/scripts/check-transcript.nu --path
#
# PROBE TRAP (fabro-7762): inside nu interpolated strings ($"...") a
# literal '(' opens a subexpression — a probe message like
# `1 line (no available-binaries content)` runs the command `no` and
# dies at runtime with "Command 'no' not found" while `nu --ide-check`
# still passes (valid syntax, runtime failure). Keep literal parens out
# of $"..." strings or build such messages with string concatenation.
#
# The wrapper streams the command's stdout/stderr to YOU unchanged and
# appends the record to the transcript; it exits with the COMMAND's own
# exit code, so failure semantics are identical to running the check
# bare. The recorded code is captured by an EXPLICIT MARKER (fabro-0577):
# the inner bash writes the guarded command's exit code to a marker file
# immediately after the command, so the recorded `-> exit N` line can
# never be the exit of a later command in this shell invocation (a
# hand-rolled `$?` echo in the check command expands at the OUTER shell
# and once recorded a green code for a failing guard). Keep the
# transcript dir constant in sync with evidence.nu's checks-section
# (shared script, two entry points — one file each side).

# Run id from the run branch; "local" off a run branch. Mirrors
# evidence.nu's current-run-id — change both together.
def current-run-id []: nothing -> string {
    let id = (git branch --show-current | parse --regex 'fabro/run/(?P<id>[^/]+)$' | get -o id.0 | default '')
    if ($id | is-empty) { "local" } else { $id }
}

# Shared transcript dir. The env override exists so the transcript
# battery can isolate its pins; production never sets it. Mirror any
# change in evidence.nu's checks-transcript-path — change both together.
def transcript-dir []: nothing -> string {
    $env.FABRO_CHECK_TRANSCRIPT_DIR? | default "/tmp/fabro-check-transcript"
}

# Pure path builder (testable without git): <dir>/<run-id>.md.
def transcript-path [run_id: string]: nothing -> string {
    $"(transcript-dir)/($run_id).md"
}

def main [...command: string, --path, --by: string = "implementer"]: nothing -> nothing {
    if $path or ($command | is-empty) {
        print (transcript-path (current-run-id))
        return
    }
    let cmd = ($command | str join ' ')
    # Explicit marker capture: append an immediate-exit block AFTER the
    # guarded command (leading newline so a trailing `# comment` in the
    # command cannot swallow it). The inner bash writes the guarded
    # command's `$?` to the marker before anything else can run, then
    # exits with it — the recorded code comes from the marker, never
    # from a `$?` read after an intervening command.
    let marker = (mktemp -t fabro-check-exit.XXXXXX)
    let guarded = $"($cmd)\n__fabro_guard_exit=\$?\nprintf '%s' \"\$__fabro_guard_exit\" > \"($marker)\"\nexit \"\$__fabro_guard_exit\"\n"
    let res = (do { bash -c $guarded } | complete)
    let recorded = (try { open --raw $marker | str trim } catch { '' })
    rm -f $marker
    let code = (if ($recorded | is-empty) {
        # Fail-open: the marker is unreadable only if bash itself died
        # before the capture; `complete` still holds that same bash's
        # exit code, and the record says which path produced the code.
        $res.exit_code
    } else {
        $recorded | into int
    })
    let out = ($res.stdout | str trim -r -c "\n")
    let err = ($res.stderr | str trim -r -c "\n")
    # Re-run stamp (fabro-16a7): a failed first attempt fixed and re-run
    # used to leave two indistinguishable records of the same command in
    # the transcript, and the reviewer had to adjudicate which one was
    # the proof of record. The wrapper now counts earlier records of the
    # SAME command in this run's transcript and stamps later ones
    # `[re-run N]`: the LATEST record of a command is the proof of
    # record, marked mechanically instead of judged from ordering.
    # Fail-open: a missing transcript or a header that does not
    # line-match (multi-line command) counts as no prior attempt —
    # the stamp is advisory and never changes the exit code.
    let tpath = (transcript-path (current-run-id))
    let attempt = (if ($tpath | path exists) {
        1 + (open --raw $tpath | lines | where {|l|
            ($l | str starts-with "$ [by ") and (($l | str replace -r '^\$ \[by [^\]]+\] ' '') == $cmd)
        } | length)
    } else { 1 })
    mut record = $"$ [by ($by)] ($cmd)\n"
    if $attempt > 1 {
        $record = $"($record)[re-run ($attempt)] supersedes earlier attempts — this latest record is the proof of record\n"
    }
    if ($out | is-not-empty) { $record = $"($record)($out)\n" }
    if ($err | is-not-empty) { $record = $"($record)[stderr] ($err)\n" }
    if ($recorded | is-empty) { $record = $"($record)[exit-fallback] marker unreadable — code from complete\n" }
    # Quoting-death hint: bash exiting 2 with no marker means the guarded
    # script never parsed — the command's quoting was mangled BEFORE the
    # wrapper saw it (nested `sh -c "..."` probes through an extra outer
    # shell layer). The wrapper cannot repair pre-arrival quoting; point
    # the caller at the canonical escape: a nu probe script under mktemp
    # (outside the worktree), wrapped as `nu /tmp/<probe>.nu`. Fail-open:
    # the hint rides the record and never changes the exit code.
    if ($recorded | is-empty) and ($res.exit_code | default 0) == 2 {
        $record = $"($record)[quoting] guarded command broke bash parsing — quoting was likely mangled before the wrapper saw it; write the probe as a nu script under `mktemp` outside the worktree and wrap `nu /tmp/<probe>.nu` instead\n"
    }
    $record = $"($record)-> exit ($code)\n\n"
    mkdir (transcript-dir)
    $record | save --append $tpath
    # Caller sees exactly what a bare run would show, plus the record
    # framing; exit code passes through so checks fail loud.
    print --no-newline ($record | str trim -r -c "\n")
    exit $code
}
