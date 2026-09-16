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
#
# Usage: nu .fabro/scripts/dup-run-check-fixtures.nu   (exit 0 = all pass)

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
    } finally {
        cd $home_dir
        rm -rf $scratch
    }
}
