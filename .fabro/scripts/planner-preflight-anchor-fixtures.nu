#!/usr/bin/env nu
# Fixture battery for planner-preflight.nu anchor verification
# (fabro-7daf): unit-drives extract-anchors / check-anchor from
# anchor_check.nu against a scratch worktree — matching anchor, rotted
# anchor (out of range), content mismatch, missing file, no-anchor seed,
# claim-bleed guard, and degradation (binary/unreadable file) — plus one
# end-to-end run of planner-preflight.nu (report-only since fabro-83df)
# whose --candidates seed id cannot be resolved by `seeds show`, proving the
# description-fetch failure degrades to "no anchors" without crashing.
# fabro-9ec3 arm 1 adds bare-path cases (10)-(12): path-only citations
# are extracted and existence-checked (workflow-relative fallback root
# included), URLs/emails/anchored needles are never misparsed as paths.
#
# Usage: nu .fabro/scripts/planner-preflight-anchor-fixtures.nu
# (exit 0 = all pass)

const FIXTURES_DIR = (path self | path dirname)
source ($FIXTURES_DIR | path join '..' 'workflows' 'develop' 'scripts' 'anchor_check.nu')

def expect [case: string, got, want] {
    if $got == $want {
        print $"PASS \($case\): ($got | to json --raw)"
    } else {
        print -e $"FAIL \($case\): got ($got | to json --raw), want ($want | to json --raw)"
        exit 1
    }
}

def main [] {
    let home_dir = $env.PWD
    let scratch = (mktemp -d -t anchor-fixtures.XXXXXX)

    try {
        cd $scratch
        mkdir pkg
        seq 1 200 | each {|i| $"line ($i) content"} | save -f pkg/a.rs
        ["doc one" "doc two" "doc three"] | save -f doc.md
        # binary blob: cited but unreadable as text
        # binary blob: cited but unreadable as text. The control bytes are
        # ESCAPES in the source, never literal: embedded here they made git
        # treat this whole battery as binary - no diffs, no review, no grep
        # (fabro-40ba).
        "binary blob \u{0}\u{1}\u{2} garbage" | save -f blob.bin

        # (1) matching anchor: file exists, line in range, claim present.
        let ok = (extract-anchors "pkg/a.rs:10 has 'line 10 content' inline" | each {|a| check-anchor $a $scratch})
        expect '1: matching anchor single status' ($ok | get status) ['ok']
        expect '1: matching anchor anchors_ok shape' ($ok | where {|f| $f.status != "ok"} | length) 0

        # (2) range anchor with claim spanning wrapped lines.
        let rng = (extract-anchors "pkg/a.rs:10-12 shows 'line 10 content line 11 content' wrapped" | each {|a| check-anchor $a $scratch})
        expect '2: range anchor status' ($rng | get status) ['ok']

        # (3) rotted anchor: line range beyond EOF.
        let rot = (extract-anchors "doc.md:99-100 rotted away" | each {|a| check-anchor $a $scratch})
        expect '3: rotted anchor status' ($rot | get status) ['out_of_range']

        # (4) content mismatch: file and range fine, claim gone.
        let mis = (extract-anchors "pkg/a.rs:20 says 'nomatch here x'" | each {|a| check-anchor $a $scratch})
        expect '4: mismatch status' ($mis | get status) ['mismatch']
        expect '4: mismatch carries claim' ($mis | first | get claim) 'nomatch here x'

        # (5) missing file.
        let gone = (extract-anchors "missing/x.rs:5 disappeared" | each {|a| check-anchor $a $scratch})
        expect '5: missing file status' ($gone | get status) ['missing_file']

        # (6) no-anchor seed: empty extraction, no flags.
        expect '6: no-anchor extraction' (extract-anchors "no anchors here, just prose and 'a quote'") []
        expect '6: empty description extraction' (extract-anchors "") []

        # (7) claim-bleed guard: the quote belongs to the SECOND anchor;
        # the first anchor must not inherit it (its own status then
        # depends only on existence/range).
        let bleed = (extract-anchors "pkg/a.rs:30 and then doc.md:2 cites 'doc two' verbatim")
        expect '7: bleed guard drops first claim' ($bleed | where start == 30 | get claim | first) ''
        expect '7: bleed guard keeps own claim' ($bleed | where start == 2 | get claim | first) 'doc two'
        expect '7: bleed guard statuses ok' ($bleed | each {|a| check-anchor $a $scratch} | get status) ['ok' 'ok']

        # (8) degradation: binary/unreadable file and multi-anchor caps
        # never crash; unreadable text degrades to out_of_range flag.
        let bin = (extract-anchors "blob.bin:2 is binary" | each {|a| check-anchor $a $scratch})
        expect '8: binary file flagged not crashed' ($bin | get status | first) 'out_of_range'

        # (9) end-to-end: planner-preflight.nu (report-only mode) with a seed
        # id `seeds show` cannot resolve — description fetch fails, anchor
        # fields degrade to "no anchors" (anchors_ok true), script exits
        # 0 and still routes "Preflight done".
        let pre = ($FIXTURES_DIR | path join '..' 'workflows' 'develop' 'scripts' 'planner-preflight.nu')
        let out = (do { echo 'RUN-ANCHFIX' | nu $pre --base origin/main --candidates fabro-anchfix-nonexistent } | complete)
        expect '9: preflight exit code' $out.exit_code 0
        let rep = ($out.stdout | lines | last | from json)
        expect '9: degraded route' $rep.preferred_next_label 'Preflight done'
        let cand = ($rep.context_updates | get 'output.preflight' | get candidates | first)
        expect '9: degraded anchors_ok' $cand.anchors_ok true
        expect '9: degraded anchor_flags empty' ($cand.anchor_flags | length) 0
        # (10) fabro-9ec3 arm 1: bare path-only citations are extracted
        # (slash-bearing only — top-level names like doc.md stay out to
        # avoid file-extension noise).
        let bp = (extract-bare-paths "see pkg/a.rs and gone/x.rs plus doc.md")
        expect '10: bare path extraction' ($bp | sort) (['gone/x.rs' 'pkg/a.rs'] | sort)

        # (11) check-bare-paths flags only nonexistent citations; a
        # workflow-relative citation resolves against
        # .fabro/workflows/develop/ (the "scripts/planner-preflight.nu"
        # class), and URLs/emails/anchored needles are never paths.
        mkdir .fabro/workflows/develop/scripts
        "x" | save -f .fabro/workflows/develop/scripts/planner-preflight.nu
        let bpcheck = (check-bare-paths "cites pkg/a.rs, missing/y.rs, scripts/planner-preflight.nu, mail a@b.io, url https://x.io/pkg/a.rs and pkg/a.rs:5 anchored" $scratch)
        expect '11: bare path flags' ($bpcheck | get path) ['missing/y.rs']
        expect '11: bare path flags carry line null' ($bpcheck | get line) [null]
        expect '11: bare path status' ($bpcheck | get status) ['missing_file']

        # (12) degradation: empty description and path-free prose yield
        # no flags; a path that exists at either root stays silent.
        expect '12: empty description bare paths' (extract-bare-paths "") []
        expect '12: no paths prose flags' (check-bare-paths "no paths, just words and 'quotes'" $scratch | length) 0

        # (13) creation-intent window: a missing bare path named right
        # after a creation verb is the seed's PROPOSED target file, not
        # rot — check-bare-paths still flags it (claim-check's advisory
        # consumes that), but the preflight arm's drop-proposed filter
        # removes it. A non-creation missing path stays flagged.
        let desc13 = "add scripts/new-hook.nu beside the existing gone/x.rs citation"
        expect '13: creation-named detected' (creation-named? $desc13 'scripts/new-hook.nu') true
        expect '13: non-creation not windowed' (creation-named? $desc13 'gone/x.rs') false
        let flags13 = (check-bare-paths $desc13 $scratch)
        expect '13: raw flags include proposed' ($flags13 | get path | sort) (['gone/x.rs' 'scripts/new-hook.nu'] | sort)
        let dropped13 = (drop-proposed $desc13 $flags13)
        expect '13: drop-proposed keeps only real rot' ($dropped13 | get path) ['gone/x.rs']
        let far = ("create " + (1..20 | each {"filler "} | str join "") + "then cite gone/x.rs as evidence")
        expect '13: distant verb does not window' (creation-named? $far 'gone/x.rs') false
        expect '13: distant verb keeps the flag' (drop-proposed $far (check-bare-paths $far $scratch) | get path) ['gone/x.rs']
    } finally {
        cd $home_dir
        rm -rf $scratch
    }
}
