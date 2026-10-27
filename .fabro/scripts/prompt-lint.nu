#!/usr/bin/env nu
# Prompt-lint for the loop workflows: prompt/graph/toml literals must not
# rot (C3 guard — generalizes fabro-41de's revisor-only check to every
# loop asset).
#
# Checks (.fabro/workflows/{conductor,develop,revisor}/ prompts + graph +
# toml, plus .fabro/scripts/*.nu):
#   1. every seed-id literal — short `fabro-<4>` and family
#      `fabro-<prefix>-<hex>` shapes, via the shared `lib/seed_id.nu`
#      predicate — resolves in the tracker (`seeds show`); an
#      unresolvable id is provenance rot. Errors.
#   2. every `justfile:<line>` anchor must point at a NON-comment line —
#      a comment line means the anchor drifted (the planner qualitygate
#      anchor rot class). Errors.
#   3. date pins `20xx-xx-xx` older than 45 days — warnings only (human
#      review; load-bearing pins are allowed to stay).
#   4. `nu -c "..."` snippets in fenced shell/bash blocks of ANY workflow's
#      prompts (`.fabro/workflows/**/prompts/*.md`) whose payload contains
#      `$` — bash double-quote interpolation strips nu variables
#      (mx-e606d5; run 01M2N36HV2). Errors; single quotes pass.
#   7. seed-id literals in ANY workflow's prompt .md files — the strict
#      id-free prompt set: prompts carry rules, never seed ids. Errors.
#   8. every prompt .md file ends with a trailing newline — a missing
#      final newline is POSIX-noncompliance rot that diff tools and
#      editors surface as noise (reviewer residual of the tracker-commands
#      split). Errors; empty files pass.
#
# Exit 1 on any error, 0 otherwise (warnings pass).

# Opt-in marker (fabro-cb5c): a fixture battery may declare that its
# `fabro-xxxx` literals are SYNTHETIC fixture ids — ids that must NOT
# resolve in the tracker. The declaration suppresses check 1 for that file
# only; a real unresolvable id in any file without the marker still errors,
# and every other check keeps running on the marked file.
# (split so THIS file's own source never contains the joined marker and
# can never self-skip its seed-id check)
const SYNTHETIC_IDS_MARKER = '# prompt-lint: ' + 'synthetic-seed-ids'

# Shared seed-id predicate: ONE accept-set for the short (fabro-e3c7) and
# family (fabro-meta-c9f2) shapes — imported, not re-implemented;
# planner-preflight's journal-claims, close-claim-check and
# salvage-analysis use the same lib.
use lib/seed_id.nu seed-ids-in

def lint-files [] {
    (glob .fabro/workflows/conductor/**/*)
    | append (glob .fabro/workflows/develop/**/*)
    | append (glob .fabro/workflows/loop/**/*)
    | append (glob .fabro/workflows/revisor/**/*)
    | append (glob .fabro/scripts/*.nu)
    | where {|p| ([$p '.md' '.fabro' '.toml' '.nu'] | any {|ext| ($p | str ends-with $ext)})}
    | where {|p| ($p | path type) == 'file'}
}

def prompt-md-files [] {
    (glob .fabro/workflows/**/prompts/*.md)
    | where {|p| ($p | path type) == 'file'}
}

# check 4: double-quoted nu -c payloads containing $ inside fenced blocks
def nu-c-dollar-errors [f] {
    let text = (open --raw $f)
    mut errors = []
    for m in ($text | parse --regex '(?s)```[\w-]*\r?\n(?<block>.*?)```') {
        for s in ($m.block | parse --regex 'nu -c "(?<payload>[^"]*)"') {
            if ($s.payload | str contains '$') {
                $errors = ($errors | append $"($f): double-quoted nu -c snippet contains \$ — use single quotes")
            }
        }
    }
    $errors
}

# check 6: provenance literals in PROMPT files — run ids and PR-number
# references are evidence, not rules (fabro-41de class; user directive
# 2026-09-19: prompts stay universal and project-agnostic — provenance
# lives in the seed Basis). Errors. Scoped to prompt .md files: scripts
# and schemas legitimately carry synthetic ids and (#n) shape docs.
def provenance-errors [f] {
    let text = (open --raw $f)
    mut errors = []
    for m in ($text | parse --regex '\brun (?<r>01M[0-9A-HJ-NP-TV-Z]{10,})') {
        $errors = ($errors | append $"($f): run id literal 'run ($m.r)' — provenance belongs in the seed Basis, not in prompts")
    }
    for m in ($text | parse --regex '\b(?<b>01M[0-9A-HJ-NP-TV-Z]{20,})\b') {
        $errors = ($errors | append $"($f): bare run id literal '($m.b)' — provenance belongs in the seed Basis, not in prompts")
    }
    for m in ($text | parse --regex '\bPR #(?<p>\d+)') {
        $errors = ($errors | append $"($f): PR-number literal 'PR #($m.p)' — provenance belongs in the seed Basis, not in prompts")
    }
    $errors
}

# check 7: seed-id literals are FORBIDDEN in prompt .md files — the
# strict id-free prompt set (the fabro-41de strict default): a closed id
# in a status-claiming sentence rots into a lie, and provenance
# citations cost attention on every stage read. Prompts state the RULE
# semantically; provenance lives in the seed Basis. Scripts/schemas
# keep the resolvable-id check 1 — synthetic fixture ids never appear
# in prompts, so no marker applies here. Errors.
def seed-id-literal-errors [f] {
    let text = (open --raw $f)
    (seed-ids-in $text) | each {|full|
        $"($f): seed id literal '($full)' in a prompt file — prompts stay id-free; state the rule semantically, provenance belongs in the seed Basis"
    }
}

# check 8: prompt .md files must end with a trailing newline — a prompt
# whose last byte is not \n leaves a POSIX-incomplete last line that
# concatenation edits and diffs surface as noise. Empty files pass.
def trailing-newline-errors [f] {
    let text = (open --raw $f)
    if (($text | str length) == 0) or ($text | str ends-with "\n") {
        []
    } else {
        [$"($f): prompt file does not end with a trailing newline"]
    }
}

# check 9 (ADR-0032 phase 3): develop AND loop workflow assets must not
# name the tracker CLI's command forms or a forge client — both lines
# run on the engine-mediated work-item tools (`fabro_work_*`), and a
# prompt or graph that names a vendor command is either rot from the
# pre-tool era or a regression. The loop lane joined the scope once the
# shared scripts stopped serving the loop lane's seeds-based nodes
# (fabro-786a, 2026-10-25: tracker-guard/claim-check left the loop
# graph, no loop node speaks the seeds protocol anymore). Scoped to
# .fabro/workflows/{develop,loop}/** EXCLUDING each workflow's
# scripts/ dir: the shared scripts (tracker-guard, claim-check,
# evidence, closeout, preflight) keep their vendor commands as the
# seeds-protocol implementation for the surfaces that still use them —
# one file plus a lane flag (ADR-0008). Errors.
# The resolved legacy-id set on the tracker: every `seeds:fabro-xxxx`
# label across the project's items, read once per lint run through gh
# (host, read-only, paginated — the facade's legacy-id seam ships with
# fork.26; until then label membership is the same answer, cutover
# stage 5, denkhaus/work#988).
# Fail-open: this is a guard-internal data fetch. A missing `gh` binary or
# an API failure throws/returns nonzero — in that case the tier suspends
# (null) instead of erroring, and every other check still runs. A missing
# external is NOT captured by `complete` (it raises), so guard with `try`.
def tracker-known-ids [] {
    let run = (try {
        do { gh api --paginate 'repos/denkhaus/work/issues?state=all&labels=project%3Afabro&per_page=100' --jq '.[].labels[].name' } | complete
    } catch {|err|
        {exit_code: 1, stdout: '', stderr: $"census unavailable: ($err.msg)"}
    })
    if $run.exit_code != 0 {
        print -e $"prompt-lint: tracker census failed — resolvable-id tier suspended this run: ($run.stderr | str trim | str substring 0..160)"
        return null
    }
    $run.stdout
    | lines
    | where {|l| ($l | str trim | str starts-with 'seeds:')}
    | each {|l| $l | str trim | str replace -r '^seeds:' ''}
}

def vendor-cli-files [] {
    (glob .fabro/workflows/develop/**/*)
    | append (glob .fabro/workflows/loop/**/*)
    | where {|p| (not ($p | str contains '/scripts/')) and ($p | path type) == 'file'}
}

def vendor-cli-errors [f] {
    let text = (open --raw $f)
    mut errors = []
    for m in ($text | parse --regex '\bseeds (?<cmd>ready|list|show|create|close|update|search)\b') {
        $errors = ($errors | append $"($f): tracker-CLI command 'seeds ($m.cmd)' in a develop/loop asset — the develop and loop lines run on the fabro_work_* tools per ADR-0032; the shared scripts' seeds-protocol commands live in scripts/, out of this check's scope")
    }
    for m in ($text | parse --regex '(?m)(?:^|\s)`?gh\s+(?<cmd>[a-z-]+)') {
        $errors = ($errors | append $"($f): forge-client command 'gh ($m.cmd)' in a develop/loop asset — no forge client anywhere in the lines' assets per ADR-0019/ADR-0032; use the run tools")
    }
    $errors
}

# check 5: routing-named top-level properties in workflow @schemas/*.json —
# warnings only: a payload containing such a field opts the node into
# routing semantics even under a custom schema (fabro-a211)
def routing-field-schema-warnings [] {
    let names = [preferred_next_label, outcome, failure_reason, suggested_next_ids, context_updates]
    mut warnings = []
    for f in (glob .fabro/workflows/**/schemas/*.json) {
        if ($f | path type) != 'file' { continue }
        let schema = (try { open --raw $f | from json } catch { null })
        if $schema == null { continue }
        let props = (try { $schema | get properties } catch { null })
        if ($props == null) or (not ($props | describe | str starts-with 'record')) { continue }
        for n in $names {
            if ($props | columns | any {|c| $c == $n }) {
                $warnings = ($warnings | append $"($f): top-level property '($n)' is routing-named — a payload containing it activates routing semantics")
            }
        }
    }
    $warnings
}

def main [] {
    let files = (lint-files)
    if ($files | is-empty) {
        print -e "prompt-lint: no files found — scope broken"
        exit 2
    }
    mut errors = []
    mut warnings = []

    # 6. provenance literals in prompts (run ids, PR refs)
    for f in (prompt-md-files) {
        $errors = ($errors | append (provenance-errors $f))
        # 7. seed-id literals in prompts — strict id-free prompt set
        $errors = ($errors | append (seed-id-literal-errors $f))
        # 8. trailing newline on prompt files
        $errors = ($errors | append (trailing-newline-errors $f))
    }

    # 9. ADR-0032 phase 3: no tracker-CLI / forge-client commands in
    # develop or loop assets (prompts, graph, toml, schemas — scripts
    # excluded: the shared scripts' seeds-protocol implementation lives
    # there by design).
    for f in (vendor-cli-files) {
        $errors = ($errors | append (vendor-cli-errors $f))
    }

    let justfile_lines = (if ('justfile' | path exists) { open --raw justfile | lines } else { [] })
    # one census for the whole run: the resolvable-id tier checks label
    # membership, not per-id lookups
    let known_ids = (tracker-known-ids)

    for f in $files {
        let text = (open --raw $f)
        # 1. seed ids must resolve — unless the file is a FIXTURE BATTERY
        # that declares synthetic fixture ids (fabro-cb5c). A battery needs
        # ids that must NOT resolve, so the marker suspends THIS check
        # alone for THIS file; every other check keeps running on it.
        #
        # Two bounds keep the suspension from becoming a silent de-gate
        # (fabro-ac84 class): the declaration must be its own TRIMMED line
        # (prose that merely mentions the marker never counts), and it is
        # honored only in a `-smoke.nu`/`-fixtures.nu` battery. The skipped
        # ids are printed, so a rotted real id in a marked file stays
        # visible instead of silently unchecked.
        let base = ($f | path basename)
        let is_battery = (($base | str ends-with '-smoke.nu') or ($base | str ends-with '-fixtures.nu'))
        let declared = ($text | lines | any {|l| ($l | str trim) == $SYNTHETIC_IDS_MARKER})
        if ($declared and (not $is_battery)) {
            $errors = ($errors | append $"($f): the synthetic-fixture-id marker is only honored in a -smoke.nu / -fixtures.nu battery")
        }
        if ($declared and $is_battery) {
            let skipped = (seed-ids-in $text)
            print $"note: ($f) declares synthetic fixture ids — resolvable-id check skipped for ($skipped | length) ids: ($skipped | str join ', ')"
        } else if $known_ids == null {
            # census failed open — the resolvable-id tier is suspended for
            # this run (warned in tracker-known-ids); do not fabricate
            # "does not resolve" errors from an empty known set
            let skipped = (seed-ids-in $text)
            if not ($skipped | is-empty) {
                print $"note: ($f): resolvable-id check skipped for ($skipped | length) ids this run"
            }
        } else {
            for full in (seed-ids-in $text) {
                if not ($full in $known_ids) {
                    $errors = ($errors | append $"($f): seed id '($full)' does not resolve in the tracker")
                }
            }
        }
        # 2. justfile anchors must not sit on comments
        for m in ($text | parse --regex 'justfile:(?<n>\d+)') {
            let n = ($m.n | into int)
            let line = (if $n < ($justfile_lines | length) { $justfile_lines | get $n } else { null })
            if $line == null {
                $errors = ($errors | append $"($f): justfile:($n) is past end of file")
            } else if (($line | str trim) | str starts-with '#') {
                $errors = ($errors | append $"($f): justfile:($n) points at a comment line — anchor drifted")
            }
        }
        # 3. old date pins warn
        for m in ($text | parse --regex '(?<d>20\d{2}-\d{2}-\d{2})') {
            let dt = ($m.d | into datetime)
            if ((date now) - $dt) > 45day {
                $warnings = ($warnings | append $"($f): date pin '($m.d)' older than 45 days — still load-bearing?")
            }
        }
    }

    # 4. double-quoted nu -c with $ in any workflow's prompt markdown
    for f in (prompt-md-files) {
        $errors = ($errors | append (nu-c-dollar-errors $f))
    }

    # 5. routing-named top-level schema properties warn
    $warnings = ($warnings | append (routing-field-schema-warnings))

    for w in ($warnings | uniq) {
        print $"warn: ($w)"
    }
    if ($errors | is-empty) {
        print $"prompt-lint: ok — ($files | length) files, ($warnings | uniq | length) warnings"
    } else {
        for e in ($errors | uniq) {
            print -e $"error: ($e)"
        }
        print -e $"prompt-lint: ($errors | uniq | length) errors"
        exit 1
    }
}
