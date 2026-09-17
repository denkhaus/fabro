#!/usr/bin/env nu
# Fixture battery for dup-run-check.nu (fabro-4b76): exercises the closure
# identity (--self) semantics end-to-end against a synthetic git repo — no
# /tmp sd wrappers, no dependence on the live tracker or the real
# merge-target branch. Fixture seed ids (fabro-fix*) do not exist in the
# tracker, so the tracker arm degrades to `unknown` and the verdicts below
# are driven purely by the landed-PR history the battery fabricates —
# exactly the closure-identity surface the seed asks to verify.
#
# Cases (seed acceptance criteria):
#   (a) landed implementation whose Fabro-Run trailer names the invoking
#       run, checked WITH --self        -> clean (self-closure, never duplicate)
#   (b) the same trailer WITHOUT --self -> duplicate (trailer is foreign)
#   (c) a landed implementation carrying a FOREIGN run's trailer, checked
#       WITH --self                     -> duplicate
#   (d)/(e) fabro-a32f filed-only: revisor-line landed PRs that merely
#       FILE a seed ("Revise run ...; file fabro-x" / "Revisor pass:
#       file N seeds") NEVER count as landed implementations -> clean
#   (f)-(h) fabro-a32f pre-planner preflight (planner-preflight.nu):
#       landed top candidate -> "Already landed"; filed-only top
#       candidate -> "Preflight done"; empty candidate list -> degraded
#       fail-open, still "Preflight done"
#
# Usage: nu .fabro/scripts/dup-run-check-fixtures.nu   (exit 0 = all pass)

# Parse-time anchor to this battery's directory (`path self` is
# parse-time only): resolves the sibling preflight script portably.
const FIXTURES_DIR = (path self | path dirname)

# Assert helper: record one case result, abort the battery on mismatch.
def expect [case: string, got, want] {
    if $got == $want {
        print $"PASS \($case\): ($got)"
    } else {
        print -e $"FAIL \($case\): got ($got), want ($want)"
        exit 1
    }
}

# Run dup-run-check for one seed id against the synthetic base and return
# the parsed JSON verdict object (one JSON line per seed id on stdout).
def check [script: path, id: string, self: string] {
    let out = (if ($self | is-empty) {
        do { nu $script $id --base origin/main } | complete
    } else {
        do { nu $script $id --base origin/main --self $self } | complete
    })
    if $out.exit_code != 0 {
        print -e $"dup-run-check exited ($out.exit_code) for ($id): ($out.stderr)"
        exit 1
    }
    $out.stdout | lines | first | from json
}


# Run planner-preflight.nu (fabro-a32f) with a piped run id against the
# synthetic base and return the parsed routing JSON object.
def preflight [script: path, candidates: string, self: string] {
    let out = (do { $self | nu $script --base origin/main --candidates $candidates --report-only } | complete)
    if $out.exit_code != 0 {
        print -e $"planner-preflight exited ($out.exit_code): ($out.stderr)"
        exit 1
    }
    $out.stdout | lines | last | from json
}

def main [] {
    let script = ('.fabro/scripts/dup-run-check.nu' | path expand)
    let home_dir = $env.PWD
    let scratch = (mktemp -d -t dup-run-check-fixtures.XXXXXX)
    let origin_dir = ($scratch | path join 'origin')

    try {
        # Synthetic upstream: bare repo the clone fetches `origin/main` from.
        ^git init -q --bare $origin_dir
        let work = ($scratch | path join 'work')
        ^git init -q $work
        cd $work
        ^git remote add origin $origin_dir
        ^git config user.email fixtures@fabro.local
        ^git config user.name 'dup-run-check fixtures'
        ^git checkout -q -b main
        ^git commit -q --allow-empty -m 'root'

        # (a)/(b) landed squash-PR implementing fabro-fix001, closed by the
        # invoking run RUN-SELF (Fabro-Run trailer names it).
        ^git commit -q --allow-empty -m 'Implement thing for fabro-fix001 (#101)' -m 'Fabro-Run: RUN-SELF'
        # (c) landed squash-PR implementing fabro-fix002, closed by a
        # FOREIGN run's trailer.
        ^git commit -q --allow-empty -m 'Implement other for fabro-fix002 (#102)' -m 'Fabro-Run: RUN-OTHER'
        ^git push -q origin main

        # Case (a): self trailer + --self -> clean, closure self, note present
        let a = (check $script 'fabro-fix001' 'RUN-SELF')
        expect 'a: self trailer with --self verdict' $a.verdict 'clean'
        expect 'a: self trailer with --self closure' ($a.implementation_matches | first | get closure) 'self'
        expect 'a: self trailer with --self closure_note' ($a.closure_note | str contains 'RUN-SELF') true

        # Case (b): same trailer, no --self -> duplicate (closure foreign)
        let b = (check $script 'fabro-fix001' '')
        expect 'b: self trailer without --self verdict' $b.verdict 'duplicate'
        expect 'b: self trailer without --self closure' ($b.implementation_matches | first | get closure) 'foreign'

        # Case (c): foreign trailer + --self -> duplicate
        let c = (check $script 'fabro-fix002' 'RUN-SELF')
        expect 'c: foreign trailer with --self verdict' $c.verdict 'duplicate'
        expect 'c: foreign trailer with --self closure' ($c.implementation_matches | first | get closure) 'foreign'

        # (d)/(e) fabro-a32f filed-only: revisor-line landed PRs that
        # merely FILE seeds never count as landed implementations.
        # ("Revise run ...; file fabro-xxx" — e.g. real a7ee183 filing
        # fabro-ea41/fabro-7aac via PR #206 — and "Revisor pass: file N
        # seeds".) Without the extended classifier these read as landed
        # implementations and would mechanically close live seeds in the
        # pre-planner preflight.
        ^git commit -q --allow-empty -m 'Revise run RUNX; file fabro-fix003 (#103)'
        ^git commit -q --allow-empty -m 'Revisor pass: file 2 seeds incl fabro-fix004 (#104)'
        ^git push -q origin main

        let d = (check $script 'fabro-fix003' 'RUN-SELF')
        expect 'd: revise-run file verdict' $d.verdict 'clean'
        expect 'd: revise-run file filed_only' $d.filed_only_matches 1

        let e = (check $script 'fabro-fix004' 'RUN-SELF')
        expect 'e: revisor file-N-seeds verdict' $e.verdict 'clean'
        expect 'e: revisor file-N-seeds filed_only' $e.filed_only_matches 1

        # (f)-(h) fabro-a32f pre-planner preflight routing (report-only:
        # no tracker writes; the close path is guarded by sd failures into
        # fail-open and is exercised by the dry-run in the seed work).
        let pre = ($FIXTURES_DIR | path join '..' 'workflows' 'develop' 'scripts' 'planner-preflight.nu')

        let f = (preflight $pre 'fabro-fix002' 'RUN-PREFLIGHT')
        expect 'f: landed top candidate route' $f.preferred_next_label 'Already landed'
        expect 'f: landed top candidate verdict' ($f.context_updates | get 'output.preflight' | get candidates | first | get verdict) 'duplicate'
        expect 'f: landed top candidate sha' ((($f.context_updates | get 'output.preflight' | get candidates | first | get sha | str length) >= 7)) true

        let g = (preflight $pre 'fabro-fix003' 'RUN-PREFLIGHT')
        expect 'g: filed-only top candidate route' $g.preferred_next_label 'Preflight done'
        expect 'g: filed-only top candidate verdict' ($g.context_updates | get 'output.preflight' | get candidates | first | get verdict) 'clean'

        let h = (preflight $pre '' 'RUN-PREFLIGHT')
        expect 'h: empty candidates route' $h.preferred_next_label 'Preflight done'
        expect 'h: empty candidates mode' ($h.context_updates | get 'output.preflight' | get mode) 'degraded'
    } finally {
        cd $home_dir
        rm -rf $scratch
    }
}
