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
# The wrapper streams the command's stdout/stderr to YOU unchanged and
# appends the record to the transcript; it exits with the COMMAND's own
# exit code, so failure semantics are identical to running the check
# bare. Keep the transcript dir constant in sync with evidence.nu's
# checks-section (shared script, two entry points — one file each side).

# Run id from the run branch; "local" off a run branch. Mirrors
# evidence.nu's current-run-id — change both together.
def current-run-id []: nothing -> string {
    let id = (git branch --show-current | parse --regex 'fabro/run/(?P<id>[^/]+)$' | get -o id.0 | default '')
    if ($id | is-empty) { "local" } else { $id }
}

# Pure path builder (testable without git): <dir>/<run-id>.md.
def transcript-path [run_id: string]: nothing -> string {
    $"/tmp/fabro-check-transcript/($run_id).md"
}

def main [...command: string, --path, --by: string = "implementer"]: nothing -> nothing {
    if $path or ($command | is-empty) {
        print (transcript-path (current-run-id))
        return
    }
    let cmd = ($command | str join ' ')
    let res = (do { bash -c $cmd } | complete)
    let out = ($res.stdout | str trim -r -c "\n")
    let err = ($res.stderr | str trim -r -c "\n")
    mut record = $"$ [by ($by)] ($cmd)\n"
    if ($out | is-not-empty) { $record = $"($record)($out)\n" }
    if ($err | is-not-empty) { $record = $"($record)[stderr] ($err)\n" }
    $record = $"($record)-> exit ($res.exit_code)\n\n"
    mkdir /tmp/fabro-check-transcript
    $record | save --append (transcript-path (current-run-id))
    # Caller sees exactly what a bare run would show, plus the record
    # framing; exit code passes through so checks fail loud.
    print --no-newline ($record | str trim -r -c "\n")
    exit $res.exit_code
}
