#!/usr/bin/env nu
# Fixture battery for the ADR-0032 phase-3 revisor rewrite: the
# closeout race is now a MANDATED PROMPT STEP (migration fork 8 retired
# closeout-race.nu), so the deterministic protection that used to live
# in the retired script's fixtures lives HERE as prompt pins: the
# freshness rule survives only if the prompts keep routing the fresh
# filing-time query. Pure file assertions — no server, no tracker CLI.
#
# Cases:
#   (a) analyze.md dedupes through the work tools: a fresh
#       query-by-run-id call routes BEFORE distillation, with a
#       covered_by_closeout drop rule
#   (b) file.md re-runs the fresh query at FILING time (before the
#       first create) and classifies a match as duplicate_of, never an
#       overflow entry
#   (c) negative: NO tracker-CLI vocabulary remains in any revisor
#       prompt (seeds create/list/search/close/update, `gh `) — the
#       tracker is engine-mediated tools only (ADR-0032; prompt-lint
#       tier guards literals, this pin guards CLI shape words)
#   (d) file.md files through fabro_work_create with the revision
#       label and reports filed_item_keys (the context-key rename)
#   (e) the overflow-ledger consume call names --item (fork 3), never
#       the retired --seed flag
#
# Usage: nu .fabro/workflows/revisor/scripts/filing-dedupe-fixtures.nu
# (exit 0 = all pass)

const DIR = ('.fabro/workflows/revisor' | path expand)

def fail [what: string]: nothing -> nothing {
    print -e $"filing-dedupe-fixtures: FAIL — ($what)"
    exit 1
}

def expect-contains [case: string, haystack: string, needle: string] {
    if ($haystack | str contains $needle) {
        print $"PASS \($case\)"
    } else {
        fail $"($case): missing '($needle)'"
    }
}

def expect-absent [case: string, haystack: string, needle: string] {
    if ($haystack | str contains $needle) {
        fail $"($case): forbidden '($needle)' present"
    } else {
        print $"PASS \($case\)"
    }
}

def main [] {
    let analyze = (open --raw ($DIR | path join 'prompts/analyze.md'))
    let file = (open --raw ($DIR | path join 'prompts/file.md'))
    let facts = (open --raw ($DIR | path join 'prompts/facts.md'))
    let select = (open --raw ($DIR | path join 'prompts/select.md'))
    let graph = (open --raw ($DIR | path join 'workflow.fabro'))

    # (a) analyze: fresh query before distilling, drop rule named.
    expect-contains 'a: analyze fresh query' $analyze 'fabro_work_list {"query": "<the reviewed run'
    expect-contains 'a: analyze drop rule' $analyze 'covered_by_closeout'
    expect-contains 'a: analyze show call' $analyze 'fabro_work_show {"itemKey"'

    # (b) file: fresh query at filing time, duplicate_of classification,
    # overflow exemption pinned.
    expect-contains 'b: file fresh query' $file 'fabro_work_list {"query": "<the reviewed run'
    expect-contains 'b: file duplicate_of' $file 'duplicate_of'
    expect-contains 'b: never overflow' $file 'NEVER an overflow entry'

    # (c) no tracker CLI / forge client vocabulary in any revisor prompt.
    for pair in [['analyze.md' $analyze] ['file.md' $file] ['facts.md' $facts] ['select.md' $select]] {
        let name = ($pair | first)
        let text = ($pair | last)
        expect-absent $"c: ($name) seeds create" $text 'seeds create'
        expect-absent $"c: ($name) seeds list" $text 'seeds list'
        expect-absent $"c: ($name) seeds search" $text 'seeds search'
        expect-absent $"c: ($name) seeds close" $text 'seeds close'
        expect-absent $"c: ($name) seeds update" $text 'seeds update'
        expect-absent $"c: ($name) gh client" $text 'gh pr'
    }

    # (d) filing contract: create via tool with the revision label, the
    # renamed context key, and the tool grant on the file node.
    expect-contains 'd: create call' $file 'fabro_work_create {"title": "...", "body": "...", "labels": ["revision"]}'
    expect-contains 'd: filed_item_keys' $file 'filed_item_keys'
    expect-contains 'd: graph file-node grant' $graph 'x.fabro_tools="fabro_work_create,fabro_work_list,fabro_work_show,fabro_work_close"'
    expect-contains 'd: graph analyze-node grant' $graph 'x.fabro_tools="fabro_ask,fabro_work_list,fabro_work_show,fabro_work_release"'
    expect-absent 'd: graph tracker write glob gone' $graph '.seeds/issues.jsonl'

    # (e) overflow-ledger consume names --item (fork 3).
    expect-contains 'e: consume --item' $file 'consume --file <file> --line <n> --item <new item key>'

    print 'filing-dedupe-fixtures: OK — fresh-query pins, CLI-free prompts, filing contract, item-key consume'
}
