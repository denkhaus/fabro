#!/usr/bin/env nu
# prompt-lint: synthetic-seed-ids
# Fixture battery for dup-run-check.nu (fabro-4b76): exercises the closure
# identity (--self) semantics end-to-end against a synthetic git repo — no
# dependence on the live tracker or the real merge-target branch. Fixture
# ids (fabro-fix-00NN) use the canonical family shape fabro-[a-z]+-[0-9a-f]{4}
# and do not exist in the tracker, so the tracker arm degrades to `unknown`
# unless a gh-state shim answers it, and the verdicts below are driven by
# the landed-PR history (and the shimmed tracker states) the battery
# fabricates — exactly the closure-identity surface the work asks to
# verify.
#
# Cases (work acceptance criteria):
#   (a) landed implementation whose Fabro-Run trailer names the invoking
#       run, checked WITH --self        -> clean (self-closure, never duplicate)
#   (b) the same trailer WITHOUT --self -> duplicate (trailer is foreign)
#   (c) a landed implementation carrying a FOREIGN run's trailer, checked
#       WITH --self                     -> duplicate
#   (d)/(e) fabro-a32f filed-only: revisor-line landed PRs that merely
#       FILE an item ("Revise run ...; file fabro-x" / "Revisor pass:
#       file N seeds") NEVER count as landed implementations -> clean
#   (i) fabro-0d48 filed-only: the comma/and-separated "file seeds
#       fabro-x and fabro-y" subject shape (e.g. real 706d639
#       'Improve: revise develop run 01M2Q7VVH, file seeds fabro-6ae9
#       and fabro-... (#212)') must classify filed-only, NOT as a
#       foreign implementation forcing a duplicate verdict -> clean
#   (o)/(p) fabro-395b bookkeeping: landed squash commits that merely
#       REOPEN ("fabro-X: reopen ... (#n)", e.g. real e769f791) or
#       VERIFY/move an item ("Verify fabro-X fix in-tree; move seed to
#       in_progress (#n)", e.g. real f3c12e03) classify filed-only,
#       NOT landed implementations -> clean (incident runs
#       01M2WG2Z42N8P2QKX0K7VXT1MC / 01M2WHTHVGK360638W9H9PN2BT)
#   (f)-(h) fabro-a32f pre-planner preflight (planner-preflight.nu):
#       landed top candidate -> "Already landed"; filed-only top
#       candidate -> "Preflight done"; empty candidate list -> degraded
#       fail-open, still "Preflight done"
#   (j) fabro-9ec3 arm 2: an id claimed on the journal of a recent,
#       UNMERGED develop run branch -> in_flight true with the run id;
#       the invoking run's own branch never marks; a branch already
#       merged into base never marks
#   (k) fabro-9ec3 arm 3: the planner output schema's brief
#       gate-command ban pattern rejects `just qualitygate` and its
#       byte-equivalent body, accepts delegated phrasing, and the
#       planner node in workflow.fabro references the schema file
#   (l)-(n) fabro-83df report-only: the script NEVER closes and NEVER
#       routes "Already landed" — duplicates (top or non-top, sha or
#       not) all route "Preflight done" as ADVISORY verdicts for the
#       planner, and the tracker states stay untouched; every report
#       carries the verdict legend. (The former fabro-ead4 live
#       close arm was retired 2026-09-19: a commit subject naming an
#       id is not closure evidence — reopen/verify commits matched too,
#       fabro-395b; incident runs 01M2WG2Z42/01M2WHTHV.)
#
# Usage: nu .fabro/scripts/dup-run-check-fixtures.nu   (exit 0 = all pass)

# Parse-time anchor to this battery's directory (`path self` is
# parse-time only): resolves the sibling preflight script portably.
const FIXTURES_DIR = (path self | path dirname)

# Assert helper: record one case result, abort the battery on mismatch.
def expect [case: string, got, want] {
    if $got == $want {
        print $"PASS ($case): ($got)"
    } else {
        print -e $"FAIL ($case): got ($got), want ($want)"
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


# Run planner-preflight.nu (fabro-a32f; report-only since fabro-83df)
# against the synthetic base and return the parsed routing JSON object.
# fabro-96c6 contract: the script takes the invoking run id from the
# FABRO_RUN_ID env (petri has no internal.run_id stdin state) — drive it
# with with-env, never a stdin pipe. A pipe was silently ignored after
# the cutover, the self-exclusion in the in-flight arm never fired, and
# case (j) regressed to naming the invoking run's own branch (the newest
# unmerged claimant) instead of the live foreign run.
# denkhaus/work#990: --candidates-file carries [{id, description}]
# records directly (the state-file pattern); descriptions travel via the
# --desc record here ({id: description}), so the fixture exercises the
# file's shape without any store round-trip.
def preflight [script: path, candidates: string, self: string, --desc: record = {}] {
    let ids = ($candidates | split row ',' | each {|c| $c | str trim} | where {|c| not ($c | is-empty)})
    let recs = ($ids | each {|id|
        {id: $id, description: ($desc | get -o $id | default "")}
    })
    let cf = '.preflight-candidates.json'
    $recs | to json --raw | save -f $cf
    let out = (do { with-env {FABRO_RUN_ID: $self} { nu $script --base origin/main --candidates-file $cf } } | complete)
    if ($out.exit_code != 0) {
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

        # (a)/(b) landed squash-PR implementing fabro-fix-0001, closed by the
        # invoking run RUN-SELF (Fabro-Run trailer names it).
        ^git commit -q --allow-empty -m 'Implement thing for fabro-fix-0001 (#101)' -m 'Fabro-Run: RUN-SELF'
        # (c) landed squash-PR implementing fabro-fix-0002, closed by a
        # FOREIGN run's trailer.
        ^git commit -q --allow-empty -m 'Implement other for fabro-fix-0002 (#102)' -m 'Fabro-Run: RUN-OTHER'
        ^git push -q origin main

        # Case (a): self trailer + --self -> clean, closure self, note present
        let a = (check $script 'fabro-fix-0001' 'RUN-SELF')
        expect 'a: self trailer with --self verdict' $a.verdict 'clean'
        expect 'a: self trailer with --self closure' ($a.implementation_matches | first | get closure) 'self'
        expect 'a: self trailer with --self closure_note' ($a.closure_note | str contains 'RUN-SELF') true

        # Case (b): same trailer, no --self -> duplicate (closure foreign)
        let b = (check $script 'fabro-fix-0001' '')
        expect 'b: self trailer without --self verdict' $b.verdict 'duplicate'
        expect 'b: self trailer without --self closure' ($b.implementation_matches | first | get closure) 'foreign'

        # (b2) work#1030 sandbox pin: a PATH without gh — every run
        # sandbox (ADR-0019 toolchain carries no gh) — must fail OPEN:
        # tracker degrades to unknown with the lookup-failed note and the
        # verdict stays history-driven (same as case b). A missing
        # external RAISES and do|complete does not catch it, so this pins
        # the tracker arm's try/catch (the prompt-lint 3f1625ba1 class;
        # the pre-fix script died here with exit 1 for fabro-fix-0001 and
        # deadlocked the loop lap run 01M5ZG7FY0V4 at its tester).
        # Wrapper scripts, not symlinks (the gh shim's own pattern): the
        # #! line is absolute, so the child PATH carries ONLY these.
        let nogh_bin = ($scratch | path join 'nogh-bin')
        mkdir $nogh_bin
        for exe in ['nu' 'git' 'sh'] {
            let real = (which $exe | first | get path)
            $"#!/bin/sh\nexec ($real) \"$@\"\n" | save -f ($nogh_bin | path join $exe)
            ^chmod +x ($nogh_bin | path join $exe)
        }
        let b2 = (with-env {PATH: $nogh_bin} { check $script 'fabro-fix-0001' '' })
        expect 'b2: gh-less tracker degrades to unknown' $b2.tracker_status 'unknown'
        expect 'b2: gh-less tracker carries the note' ($b2.tracker_note | str contains 'tracker lookup failed') true
        expect 'b2: gh-less verdict stays history-driven' $b2.verdict 'duplicate'

        # Case (c): foreign trailer + --self -> duplicate
        let c = (check $script 'fabro-fix-0002' 'RUN-SELF')
        expect 'c: foreign trailer with --self verdict' $c.verdict 'duplicate'
        expect 'c: foreign trailer with --self closure' ($c.implementation_matches | first | get closure) 'foreign'

        # (d)/(e) fabro-a32f filed-only: revisor-line landed PRs that
        # merely FILE seeds never count as landed implementations.
        # ("Revise run ...; file fabro-xxx" — e.g. real a7ee183 filing
        # fabro-ea41/fabro-7aac via PR #206 — and "Revisor pass: file N
        # seeds".) Without the extended classifier these read as landed
        # implementations and would mechanically close live seeds in the
        # pre-planner preflight.
        ^git commit -q --allow-empty -m 'Revise run RUNX; file fabro-fix-0003 (#103)'
        ^git commit -q --allow-empty -m 'Revisor pass: file 2 seeds incl fabro-fix-0004 (#104)'
        ^git push -q origin main

        let d = (check $script 'fabro-fix-0003' 'RUN-SELF')
        expect 'd: revise-run file verdict' $d.verdict 'clean'
        expect 'd: revise-run file filed_only' $d.filed_only_matches 1

        let e = (check $script 'fabro-fix-0004' 'RUN-SELF')
        expect 'e: revisor file-N-seeds verdict' $e.verdict 'clean'
        expect 'e: revisor file-N-seeds filed_only' $e.filed_only_matches 1

        # (i) fabro-0d48: the 706d639 shape — 'Improve: revise develop
        # run <id>, file seeds fabro-x and fabro-y (#n)' — previously
        # matched NO classifier branch (revisors wrote 'revise develop
        # run', and 'file seeds fabro-' was uncovered), so it landed
        # among implementations with closure=foreign and line ~244's
        # foreign_impl > 0 forced verdict=duplicate even when the run's
        # own self-closure PR existed. Both mentioned seeds must read
        # filed-only -> clean.
        ^git commit -q --allow-empty -m 'Improve: revise develop run RUN-REV, file seeds fabro-fix-0005 and fabro-fix-0006 (#105)'
        ^git push -q origin main

        let i1 = (check $script 'fabro-fix-0005' 'RUN-SELF')
        expect 'i: file-seeds-and verdict (first seed)' $i1.verdict 'clean'
        expect 'i: file-seeds-and filed_only (first seed)' $i1.filed_only_matches 1
        expect 'i: file-seeds-and impl matches (first seed)' ($i1.implementation_matches | length) 0

        let i2 = (check $script 'fabro-fix-0006' 'RUN-SELF')
        expect 'i: file-seeds-and verdict (second seed)' $i2.verdict 'clean'
        expect 'i: file-seeds-and filed_only (second seed)' $i2.filed_only_matches 1
        expect 'i: file-seeds-and impl matches (second seed)' ($i2.implementation_matches | length) 0

        # (o)/(p) fabro-395b: landed squash commits that merely REOPEN or
        # VERIFY/move the named seed (the real 2026-09-19 incident shapes:
        # e769f791 'fabro-29f7: reopen gate seed, verify upstream offers
        # remain unmerged ... (#250)' and f3c12e03 'Verify fabro-6a78 fix
        # in-tree; move seed to in_progress ... (#254)') must classify
        # filed-only, never landed implementations — otherwise the
        # pre-planner preflight mechanically re-closes deliberately
        # reopened seeds.
        ^git commit -q --allow-empty -m 'fabro-fix-0013: reopen gate seed, verify upstream offers remain unmerged (#109)'
        ^git commit -q --allow-empty -m 'Verify fabro-fix-0014 fix in-tree; move seed to in_progress (#110)'
        ^git push -q origin main

        let o = (check $script 'fabro-fix-0013' 'RUN-SELF')
        expect 'o: reopen squash verdict' $o.verdict 'clean'
        expect 'o: reopen squash filed_only' $o.filed_only_matches 1
        expect 'o: reopen squash impl matches' ($o.implementation_matches | length) 0

        let p = (check $script 'fabro-fix-0014' 'RUN-SELF')
        expect 'p: verify/move-seed squash verdict' $p.verdict 'clean'
        expect 'p: verify/move-seed squash filed_only' $p.filed_only_matches 1
        expect 'p: verify/move-seed squash impl matches' ($p.implementation_matches | length) 0

        # (q)/(r) fabro-4401 mention-only: a landed squash whose ONLY
        # seed-id reference sits on a journal-style bullet of the commit
        # body (the incident shape: cherry-pick a7f7b2d4b8 #356 vs run
        # 01M3YWBVKHAC8EHPYWM9BQBXZ3, whose diff was entirely foreign)
        # is a MENTION, never a landed implementation -> clean with a
        # mention_only count. A real implementation whose subject uses the
        # short-id form and whose body carries the id on a NARRATIVE line
        # ("Root cause (fabro-x): ...", e.g. real 9df5f5c4f 'fix(2093):
        # ...') still matches -> duplicate.
        ^git commit -q --allow-empty -m 'cherry-pick #911: foreign fix lands here (#111)' -m '* sprint ledger bootstrap (ADR-0024, fabro-fix-0015 skeleton): scope narrowed'
        ^git commit -q --allow-empty -m 'fix(16): automation edit keeps overlap policy (#112)' -m 'Root cause (fabro-fix-0016): the replace payload could not carry the field'
        ^git push -q origin main

        let q = (check $script 'fabro-fix-0015' 'RUN-SELF')
        expect 'q: bullet-only mention verdict' $q.verdict 'clean'
        expect 'q: bullet-only mention count' $q.mention_only_matches 1
        expect 'q: bullet-only mention impl matches' ($q.implementation_matches | length) 0
        expect 'q: bullet-only mention filed_only count' $q.filed_only_matches 0

        let rr = (check $script 'fabro-fix-0016' 'RUN-SELF')
        expect 'r: narrative-body implementation verdict' $rr.verdict 'duplicate'
        expect 'r: narrative-body implementation matches' ($rr.implementation_matches | length) 1
        expect 'r: narrative-body not mention-only' $rr.mention_only_matches 0

        # (l)-(n) fabro-ead4 fixture history: foreign landed
        # implementations for fix010 (closes from the live arm, non-top),
        # fix011 (duplicate verdict but deliberately NO tracker row -> its
        # close aborts fail-open), and fix012 (top-position close -> the
        # real "Already landed" route). fix009 gets NO commit at all: it
        # is tracker-closed with no landed evidence — the ambiguous
        # duplicate that must stay for planner adjudication.
        ^git commit -q --allow-empty -m 'Implement dup-close arm for fabro-fix-0010 (#106)' -m 'Fabro-Run: RUN-OTHER'
        ^git commit -q --allow-empty -m 'Implement dup-close arm for fabro-fix-0011 (#107)' -m 'Fabro-Run: RUN-OTHER'
        ^git commit -q --allow-empty -m 'Implement dup-close arm for fabro-fix-0012 (#108)' -m 'Fabro-Run: RUN-OTHER'
        ^git push -q origin main

        # (f)-(h) fabro-a32f pre-planner preflight routing (report-only
        # since fabro-83df: no tracker writes EVER; duplicates are
        # advisory planner input, never exits or closes).
        let pre = ($FIXTURES_DIR | path join '..' 'workflows' 'develop' 'scripts' 'planner-preflight.nu')

        let f = (preflight $pre 'fabro-fix-0002' 'RUN-PREFLIGHT')
        expect 'f: landed top candidate routes to planner (report-only)' $f.preferred_next_label 'Preflight done'
        expect 'f: landed top candidate verdict' ($f.context_updates | get 'output.preflight' | get candidates | first | get verdict) 'duplicate'
        expect 'f: landed top candidate sha' ((($f.context_updates | get 'output.preflight' | get candidates | first | get sha | str length) >= 7)) true

        let g = (preflight $pre 'fabro-fix-0003' 'RUN-PREFLIGHT')
        expect 'g: filed-only top candidate route' $g.preferred_next_label 'Preflight done'
        expect 'g: filed-only top candidate verdict' ($g.context_updates | get 'output.preflight' | get candidates | first | get verdict) 'clean'

        let h = (preflight $pre '' 'RUN-PREFLIGHT')
        expect 'h: empty candidates route' $h.preferred_next_label 'Preflight done'
        expect 'h: empty candidates mode' ($h.context_updates | get 'output.preflight' | get mode) 'degraded'

        # (j) fabro-9ec3 arm 2: run-branch -> journal -> seed-id mapping.
        # RUN-LIVE claims fabro-fix-0007 on its journal and is NOT merged
        # into main -> in flight. The invoking run's own branch also
        # claims fabro-fix-0007 (self must never mark). RUN-DONE claims
        # fabro-fix-0008 but sits at main's tip (merged ancestor) -> not
        # in flight; that seed is the landed arm's business.
        ^git checkout -q -b run-live
        mkdir .fabro/journal
        ('{"$schema":"fabro-journal-v1","run_id":"RUN-LIVE","node":"planner","visit":1,"status":"succeeded","ts":"2026-09-17T00:00:00Z","data":{"painpoints":[],"observations":["fabro-fix-0007 claimed; preflight clean"]}}' | save -f .fabro/journal/RUN-LIVE.jsonl)
        ^git add .fabro
        ^git commit -q -m 'journal RUN-LIVE'
        ^git push -q origin HEAD:refs/heads/fabro/run/RUN-LIVE
        ^git checkout -q -b run-self
        ('{"$schema":"fabro-journal-v1","run_id":"RUN-PREFLIGHT","node":"planner","visit":1,"status":"succeeded","ts":"2026-09-17T00:00:00Z","data":{"painpoints":[],"observations":["fabro-fix-0007 claimed by self"]}}' | save -f .fabro/journal/RUN-PREFLIGHT.jsonl)
        ^git add .fabro
        ^git commit -q -m 'journal RUN-PREFLIGHT'
        ^git push -q origin HEAD:refs/heads/fabro/run/RUN-PREFLIGHT
        # run-done branches from MAIN (not run-self): its journal commit
        # must not drag RUN-LIVE's tip into main's history via the ff
        # below — RUN-LIVE has to stay unmerged for the in-flight mark.
        ^git checkout -q main
        ^git checkout -q -b run-done
        mkdir .fabro/journal
        ('{"$schema":"fabro-journal-v1","run_id":"RUN-DONE","node":"planner","visit":1,"status":"succeeded","ts":"2026-09-17T00:00:00Z","data":{"painpoints":[],"observations":["fabro-fix-0008 claimed but merged"]}}' | save -f .fabro/journal/RUN-DONE.jsonl)
        ^git add .fabro
        ^git commit -q -m 'journal RUN-DONE'
        ^git push -q origin HEAD:refs/heads/fabro/run/RUN-DONE
        # Fast-forward main over run-done's journal commit: run-done is
        # now a true ancestor of base while its journal still claims
        # fabro-fix-0008 — merged branches must never mark in flight.
        ^git checkout -q main
        ^git merge -q --ff-only run-done
        ^git push -q origin main
        ^git checkout -q main

        let j1 = (preflight $pre 'fabro-fix-0007' 'RUN-PREFLIGHT')
        let jrow = ($j1.context_updates | get 'output.preflight' | get candidates | first)
        expect 'j: unmerged foreign run marks in_flight' $jrow.in_flight true
        expect 'j: in_flight_run names the live run' $jrow.in_flight_run 'RUN-LIVE'

        let j2 = (preflight $pre 'fabro-fix-0008' 'RUN-PREFLIGHT')
        let j2row = ($j2.context_updates | get 'output.preflight' | get candidates | first)
        expect 'j: merged run branch never marks' $j2row.in_flight false

        # (l)-(n) fabro-83df report-only: TWO tracker surfaces now —
        # dup-run-check resolves each candidate's legacy label through gh
        # (cutover stage 5, denkhaus/work#988; the battery plants a gh
        # shim whose answers live in state files, one per fixture id,
        # content open/closed). planner-preflight's candidate descriptions
        # travel in the candidates file itself (the denkhaus/work#990
        # shape) — no store round-trip anywhere. The script must NEVER
        # mutate either surface: duplicates of every shape route
        # "Preflight done" as advisory verdicts and the planner owns
        # decisions and closures.
        let gh_dir = ($scratch | path join 'gh-states')
        let gh_bin = ($scratch | path join 'bin')
        mkdir $gh_dir
        mkdir $gh_bin
        ('open' | save ($gh_dir | path join 'fabro-fix-0003'))
        ('closed' | save ($gh_dir | path join 'fabro-fix-0009'))
        ('open' | save ($gh_dir | path join 'fabro-fix-0010'))
        ('open' | save ($gh_dir | path join 'fabro-fix-0012'))
        (
    '#!/bin/sh
# fixture gh shim: answers the legacy-label state lookup from state files
for id in $(ls "$GH_STATES" 2>/dev/null); do
  case "$*" in *"seeds:$id"*) cat "$GH_STATES/$id"; exit 0;; esac
done
echo missing'
            | save ($gh_bin | path join 'gh')
        )
        ^chmod +x ($gh_bin | path join 'gh')
        $env.PATH = ($env.PATH | prepend $gh_bin)
        $env.GH_STATES = $gh_dir
        def gh-state [id: string] {
          open --raw ($gh_dir | path join $id)
        }

        # (l) mixed batch: clean top, trackerless duplicate, non-top
        # duplicate — all route "Preflight done", nothing closes, no
        # close_log exists, mode stays "checked" (no aborts possible:
        # the report-only script has no failing side effects).
        let l = (preflight $pre 'fabro-fix-0003,fabro-fix-0011,fabro-fix-0010' 'RUN-PREFLIGHT')
        expect 'l: mixed batch routes to planner' $l.preferred_next_label 'Preflight done'
        let lrep = ($l.context_updates | get 'output.preflight')
        expect 'l: no closed field in report' ($lrep | get -o closed | default null) null
        expect 'l: no close_log field in report' ($lrep | get -o close_log | default null) null
        expect 'l: mode stays checked' $lrep.mode 'checked'
        expect 'l: trackerless duplicate verdict still duplicate' ($lrep.candidates | where {|c| $c.seed == 'fabro-fix-0011'} | first | get verdict) 'duplicate'
        expect 'l: legend maps duplicate' ($lrep.legend.duplicate | str contains 'merge-target base') true
        expect 'l: legend maps clean' ($lrep.legend.clean | str contains 'no landed implementation') true
        expect 'l: legend maps degraded' ($lrep.legend.degraded | str contains 'check failed') true
        expect 'l: clean top stays open' (gh-state 'fabro-fix-0003') 'open'
        expect 'l: non-top duplicate stays open' (gh-state 'fabro-fix-0010') 'open'

        # (m) ambiguous duplicate: fix009 is tracker-closed with no
        # landed implementation and no closing evidence — verdict
        # duplicate without a sha. Report-only: it is reported as-is,
        # nothing closes, the tracker row stays byte-identical.
        let m = (preflight $pre 'fabro-fix-0009' 'RUN-PREFLIGHT')
        expect 'm: ambiguous top routes to planner' $m.preferred_next_label 'Preflight done'
        let mrep = ($m.context_updates | get 'output.preflight')
        expect 'm: ambiguous duplicate verdict reported' ($mrep.candidates | first | get verdict) 'duplicate'
        expect 'm: ambiguous duplicate sha null' ($mrep.candidates | first | get sha) null
        expect 'm: tracker state file untouched' (gh-state 'fabro-fix-0009') 'closed'

        # (m2) the surviving body-grep closing-evidence arm (work#1477):
        # tracker-closed, NO landed implementation, but a base-ref commit
        # names the id in its BODY — resolve-closing reports that commit
        # as closing_evidence (source "body-grep") with its own trailer's
        # closure identity. A plain (non-PR) commit never counts as an
        # implementation, so the arm fires instead of an impl match.
        ('closed' | save ($gh_dir | path join 'fabro-fix-0040'))
        ^git commit -q --allow-empty -m 'notes: context for fabro-fix-0040' -m 'Root cause (fabro-fix-0040): tracked in the tracker' -m 'Fabro-Run: RUN-OTHER'
        ^git push -q origin main
        let m2 = (check $script 'fabro-fix-0040' 'RUN-SELF')
        expect 'm2: tracker-closed with body evidence verdict' $m2.verdict 'duplicate'
        expect 'm2: closing evidence resolves body-grep' ($m2.closing_evidence | get -o source) 'body-grep'
        expect 'm2: closing evidence names the commit' (($m2.closing_evidence | get -o sha | default '' | str length) >= 7) true
        expect 'm2: closing evidence closure is foreign' ($m2.closing_evidence | get -o closure) 'foreign'
        expect 'm2: closing evidence carries the trailer' ($m2.closing_evidence | get -o trailer_run) 'RUN-OTHER'

        # (n) top-position duplicate with resolvable sha: the strongest
        # already-landed signal that USED to auto-close — it still only
        # routes "Preflight done"; the tracker row stays open for the
        # planner's two-branch-rule decision.
        let n = (preflight $pre 'fabro-fix-0012' 'RUN-PREFLIGHT')
        expect 'n: top duplicate routes to planner' $n.preferred_next_label 'Preflight done'
        let nrep = ($n.context_updates | get 'output.preflight')
        expect 'n: duplicate sha still reported' (($nrep.candidates | first | get sha | default null | str length) >= 7) true
        expect 'n: tracker shows top seed still open' (gh-state 'fabro-fix-0012') 'open'

        # (s1)-(s5) fabro-a8d5 stranded-implementation scan: salvage
        # pointers in candidate descriptions are verified mechanically —
        # resolved commit present, NOT already in base, source branch
        # still fetched — and a salvage-source run branch cited WITHOUT
        # the canonical `salvage: git show <commit> on branch <branch>`
        # bullet is flagged pointer_missing (complements the planner
        # salvage-pointer rule; the planner resolves the commit before
        # shipping the brief).
        ^git checkout -q -b run-strand
        ^git commit -q --allow-empty -m 'stranded implementer commit for scan fixtures'
        let strand_sha = (^git rev-parse HEAD | str trim)
        ^git push -q origin HEAD:refs/heads/fabro/run/RUN-STRAND
        ^git checkout -q main
        let in_base_sha = (^git rev-parse HEAD | str trim)
        let sdesc = {
            fabro-fix-0020: ("Basis: run RUN-STRAND. salvage: git show " + $strand_sha + " on branch fabro/run/RUN-STRAND")
            fabro-fix-0021: "salvage: git show 0123456789abcdef on branch fabro/run/RUN-STRAND"
            fabro-fix-0022: ("salvage: git show " + $in_base_sha + " on branch fabro/run/RUN-STRAND")
            fabro-fix-0023: "salvage from stranded branch fabro/run/RUN-STRAND, commit unresolved"
            fabro-fix-0024: "no salvage surface at all, just prose"
        }
        let s = (preflight $pre 'fabro-fix-0020,fabro-fix-0021,fabro-fix-0022,fabro-fix-0023,fabro-fix-0024' 'RUN-PREFLIGHT' --desc $sdesc)
        let srep = ($s.context_updates | get 'output.preflight')
        let srow_of = {|id| $srep.candidates | where {|c| $c.seed == $id} | first}
        expect 's1: resolved unmerged pointer salvage_ok' (do $srow_of fabro-fix-0020 | get salvage_ok) true
        expect 's1: resolved unmerged pointer no flags' (do $srow_of fabro-fix-0020 | get salvage_flags | length) 0
        expect 's2: unresolvable commit flagged' (do $srow_of fabro-fix-0021 | get salvage_flags | first | get problem) 'commit_missing'
        expect 's2: unresolvable commit salvage_ok false' (do $srow_of fabro-fix-0021 | get salvage_ok) false
        expect 's3: already-in-base salvage flagged' (do $srow_of fabro-fix-0022 | get salvage_flags | first | get problem) 'already_in_base'
        expect 's4: pointerless stranded branch flagged' (do $srow_of fabro-fix-0023 | get salvage_flags | first | get problem) 'pointer_missing'
        expect 's4: pointerless branch named' (do $srow_of fabro-fix-0023 | get salvage_flags | first | get branch) 'fabro/run/RUN-STRAND'
        expect 's5: salvage-free seed salvage_ok' (do $srow_of fabro-fix-0024 | get salvage_ok) true
        expect 's5: salvage-free seed no flags' (do $srow_of fabro-fix-0024 | get salvage_flags | length) 0
        expect 's: legend maps stranded_implementation' ($srep.legend | get -o stranded_implementation | default '' | str contains 'salvage') true

        # (k) fabro-9ec3 arm 3: schema ban semantics + graph wiring.
        let wf = ($FIXTURES_DIR | path join '..' 'workflows' 'develop')
        let schema_ok = (do { ^python3 -c ("import json,re;s=json.load(open('" + ($wf | path join 'schemas' 'planner-output.schema.json') + "'));p=re.compile(s['properties']['context_updates']['properties']['current_seed_brief']['pattern']);assert p.search('gate green via the deterministic tester step');assert not p.search('run just qualitygate');assert not p.search('invoke nu scripts/qualitygate.nu');print('ok')") } | complete)
        expect 'k: schema ban pattern semantics' ($schema_ok.stdout | str trim) 'ok'
        let graph = (open --raw ($wf | path join 'workflow.fabro'))
        expect 'k: planner node wired to schema' ($graph | str contains 'output_schema="@schemas/planner-output.schema.json"') true
    } finally {
        cd $home_dir
        rm -rf $scratch
    }
}
