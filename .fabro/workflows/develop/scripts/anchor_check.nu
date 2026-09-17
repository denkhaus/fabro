# Cited file:line anchor extraction + verification (fabro-7daf).
#
# Seed descriptions cite their evidence as `path:line` / `path:line-line`
# anchors (e.g. `scripts/evidence.nu:431`, `operations/create.rs:556-585`),
# often with the claimed content quoted right after the anchor
# ("at planner-preflight.nu:135 the declaration 'mut closed = ...'").
# Anchor rot — file gone, lines shifted out of range, cited content
# rewritten — is a strong dead-seed signal the planner previously had to
# re-derive by hand. These helpers extract and verify anchors so the
# preflight verdict table can carry an `anchors_ok` field mechanically.
#
# Fail-open: every failure degrades to "no information" (empty anchor
# list, or existence/range-only checking) — never a crash. Callers route
# candidates with flagged anchors for planner adjudication instead of
# acting on the flags themselves.
#
# Path resolution is relative to the CURRENT worktree root (passed in by
# the caller); loop-asset paths under `.fabro/` etc. are readable here
# because fs_hide binds file TOOLS, not shell/script reads.

# Collapse whitespace so quoted claims match across line wrapping.
export def norm-ws [s: string] {
    $s | str replace --all --regex '\s+' ' ' | str trim
}

# Extract {path, start, end, claim} anchor records from a seed
# description. The optional claim is the first backtick/single-quoted
# snippet of >= 8 chars within 240 chars AFTER the anchor — the usual
# "at path:N the code 'foo = bar'" citation shape. A claim is only
# paired when its quote appears BEFORE any other anchor citation in the
# scan window (positional bleed guard), so multi-anchor descriptions do
# not attach one seed's quote to another anchor.
export def extract-anchors [desc: string] {
    if ($desc | is-empty) { [] } else {
        let q = "[\u{60}']"
        let claim_pat = ("^[^" + $q + "]{0,160}?" + $q + "(?P<claim>[^" + $q + "]{5,240})" + $q)
        let raw = ($desc | parse --regex "(?P<path>[A-Za-z0-9_./@-]+\\.[A-Za-z]{1,8}):(?P<start>\\d+)(?:-(?P<end>\\d+))?")
        let dlen = ($desc | str length)
        let anchors = ($raw | each {|m|
            let e = (if ($m.end | is-empty) { $m.start } else { $m.end })
            let needle = ($m.path + ":" + $m.start + (if ($m.end | is-empty) { "" } else { "-" + $m.end }))
            let idx = ($desc | str index-of $needle)
            let from = ($idx + ($needle | str length))
            let to = ([($from + 240) $dlen] | math min)
            let tail = (if $idx < 0 { "" } else { $desc | str substring $from..$to })
            {path: $m.path, start: ($m.start | into int), end: ($e | into int), needle: $needle, tail: $tail}
        })
        let needles = ($anchors | get needle)
        $anchors | each {|a|
            let qpos = (try { [($a.tail | str index-of "'") ($a.tail | str index-of "\u{60}")] | where {|i| $i >= 0} | math min } catch { null })
            let npos = (try { $needles | where {|n| $n != $a.needle} | each {|n| $a.tail | str index-of $n } | where {|i| $i >= 0} | math min } catch { null })
            let claim = (if $qpos != null and ($npos == null or $qpos < $npos) {
                $a.tail | parse --regex $claim_pat | get -o claim | default [] | first | default ""
            } else { "" })
            {path: $a.path, start: $a.start, end: $a.end, claim: $claim}
        } | uniq | first 40
    }
}

# Verify one anchor against the current worktree. Returns a flag record:
#   ok           file exists, lines in range, claim (if any) still present
#   missing_file cited path does not exist
#   out_of_range line(s) beyond EOF (or unreadable/non-UTF8 file)
#   mismatch     cited lines no longer contain the quoted claim
export def check-anchor [a: record, root: string] {
    let p = ($root | path join $a.path)
    if not ($p | path exists) {
        {path: $a.path, line: $a.start, status: "missing_file"}
    } else {
        let ls = (try { open --raw $p | lines } catch { [] })
        let n = ($ls | length)
        if $n == 0 or $a.start < 1 or $a.end > $n {
            {path: $a.path, line: $a.start, status: "out_of_range"}
        } else {
            let cited = ($ls | skip ($a.start - 1) | take ($a.end - $a.start + 1) | str join " ")
            if ($a.claim | is-empty) or ((norm-ws $cited) | str contains (norm-ws $a.claim)) {
                {path: $a.path, line: $a.start, status: "ok"}
            } else {
                {path: $a.path, line: $a.start, status: "mismatch", claim: $a.claim}
            }
        }
    }
}

# Bare repo-path extraction (fabro-9ec3 arm 1 — closes fabro-4c81's gap
# that fabro-7daf left open): seed bodies also cite paths WITHOUT a line
# anchor ("the publish step of .fabro/workflows/develop/workflow.fabro"),
# and path-only citations were never existence-checked — exactly the
# wrong-path class that drove run 01M2NA1Z3HS7QPQR8AEWZR3GDB's planner
# burn. extract-bare-paths strips URLs, emails, and the needles of
# already-verified path:line anchors first, then collects slash-bearing
# file paths; check-bare-paths flags nonexistent ones as missing_file
# with line null, folded into the SAME anchor_flags channel — an
# extension of the existing check, not a parallel table. Same fail-open
# contract: any parse surprise degrades to no flags.
export def extract-bare-paths [desc: string] {
    if ($desc | is-empty) { [] } else {
        # extract-anchors returns {path,start,end,claim} (no raw needle);
        # reconstruct the needles so they can be blanked before bare-path
        # scanning — a path:line citation must not double-report.
        let anchored = (extract-anchors $desc | each {|a|
            $a.path + ":" + ($a.start | into string) + (if $a.end > $a.start { "-" + ($a.end | into string) } else { "" })})
        mut cleaned = ($desc
            | str replace --all --regex '\S+://\S+' ' '
            | str replace --all --regex '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}' ' ')
        for n in $anchored {
            $cleaned = ($cleaned | str replace --all $n ' ')
        }
        $cleaned
        | parse --regex '(?<![A-Za-z0-9_./@-])(?P<path>[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.@-]+)+\.[A-Za-z]{1,8})'
        | get -o path
        | default []
        | uniq
        | first 40
    }
}

# Existence-only verification for bare paths: nonexistent citation ->
# {path, line: null, status: "missing_file"}; existing paths stay silent
# (no line content to compare against).
export def check-bare-paths [desc: string, root: string] {
    # Seed bodies cite paths both repo-rooted (lib/.../main.rs) and
    # workflow-relative ("prompts/planner.md", "scripts/planner-preflight.nu"
    # — relative to .fabro/workflows/develop/). Resolve against both roots
    # before flagging, so a legitimate relative citation is not rot.
    let roots = [$root ($root | path join '.fabro' 'workflows' 'develop')]
    extract-bare-paths $desc | each {|p|
        if ($roots | any {|r| $r | path join $p | path exists}) { null } else { {path: $p, line: null, status: "missing_file"} }
    } | compact
}
