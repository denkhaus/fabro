#!/usr/bin/env nu
# archive-seeds.nu — runbook stage 4 (fabro-4fb4): archive a source repo's
# frozen .seeds files byte-exact into the tracker repository, so the tracker
# is the one place for items AND history.
#
# Archive layout (user directive 2026-10-25): ONE SUBFOLDER PER SOURCE REPO,
# named after the original repo — this migration is a TEMPLATE for other
# seed-tracker repos moving onto fabro, and each source lands in its own
# namespace:
#   <tracker>/archive/<owner>-<repo>/issues-<freeze-date>.jsonl
#   <tracker>/archive/<owner>-<repo>/plans-<freeze-date>.jsonl
#
# The archive is the byte-exact history; the tracker items are the canonical
# PROJECTION (import-work-items.nu). Deleting the in-tree .seeds afterwards
# is a separate, explicit step (the in-repo deletion commit).
#
# Usage:
#   nu .fabro/scripts/archive-seeds.nu --tracker denkhaus/work \
#     --source-repo denkhaus/fabro --date 2026-10-25 \
#     --from .seeds --files issues.jsonl,plans.jsonl [--apply 1]

def main [
    --tracker: string = 'denkhaus/work'   # archive destination repo
    --source-repo: string = 'denkhaus/fabro'  # the repo whose .seeds this is
    --date: string                        # freeze date (file suffix)
    --from: string = '.seeds'             # local .seeds directory
    --files: string = 'issues.jsonl,plans.jsonl'
    --apply: int = 0                      # 0 = show plan, 1 = write
]: nothing -> nothing {
    if ($date | is-empty) { fail 'archive: --date is required (the freeze date, YYYY-MM-DD)' }
    let folder = ($source_repo | str replace '/' '-')
    let src_name = $source_repo
    for $name in ($files | split row ',') {
        let local = ($from | path join $name)
        if not ($local | path exists) { fail $"archive: ($local) not found" }
        let stem = ($name | str replace '.jsonl' '')
        let remote = $"archive/($folder)/($stem)-($date).jsonl"
        let bytes = (open --raw $local | encode base64)
        print $"archive: ($local) [($bytes | str length) b64 chars] -> ($tracker)/($remote)"
        if $apply == 0 { continue }
        let payload = ({message: $"archive: ($src_name) ($name) at freeze ($date) [fabro-4fb4]", content: $bytes} | to json)
        let run = ($payload | do { gh api --method PUT $"repos/($tracker)/contents/($remote)" --input - } | complete)
        if $run.exit_code != 0 {
            print -e $"archive: PUT ($remote) failed: ($run.stderr | str trim | str substring 0..200)"
            exit 1
        }
        print $"  ok: ($run.stdout | from json | get content | get html_url)"
    }
    if $apply == 0 { print 'archive: plan shown; rerun with --apply 1 to write' }
}

def fail [msg: string]: nothing -> nothing {
    print -e $"╔══ ARCHIVE ALARM: ($msg) ══╗"
    exit 1
}
