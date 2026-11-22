#!/usr/bin/env nu
# lint-tool-grants — the tool allow-list admission lint (work#1624), the
# safety net for the #1303 class.
#
# The failure it catches: a workflow prompt names a fabro_* run tool the
# node's x.fabro_tools grant does not admit. The denial is enforced at
# call time, so the agent pays for it in discovery rounds mid-stage and
# the stage silently degrades — work#1303 is the on-record case: the
# develop reviewer's evidence protocol COMMANDED fabro_blob and named
# fabro_work_show while the grant list had dropped them, and the reviewer
# died hunting files the engine never materialized instead of reviewing.
#
# Shape: for every .fabro/workflows/*/workflow.fabro, parse the agent
# nodes (a prompt attribute plus an x.fabro_tools grant list), resolve the
# prompt file (following its {% include %} references — the prompt the
# agent sees is the composite), scan for fabro_[a-z_]+ tokens, and compare:
#   HARD     — the prompt names a tool the grant does not admit. Exit 1.
#   ADVISORY — the grant admits a tool the prompt never names. Drift or
#              engine bookkeeping; printed, exit stays 0.
#   exempt   — a HARD mention covered by the structural exemption table
#              below: prompts legitimately quote tool names as
#              documentation for OTHER nodes. Printed for transparency.
#
# Exit code design: HARD findings -> 1; advisory-only -> 0 with the
# advisories printed; broken scope (no graphs) -> 2. The battery runner
# executes this script, so a HARD finding reds the gate exactly where a
# prompt/grant drift would otherwise reach production runs silently.

# Structural HARD exemptions — mentions that are documentation, not use.
# Each entry must name the graph, the node, the tool, and WHY the mention
# is legitimate. Keep this table minimal: an entry here is a promise that
# the node will never CALL the tool on its own behalf.
const HARD_EXEMPTS = [
    {
        graph: 'develop'
        node: 'implementer'
        tool: 'fabro_work_next'
        why: "documents the Planner's atomic claim grant as a do-not-use boundary - the sentence tells the implementer the Planner owns the item tools and must not claim, close, release, or edit"
    }
]

# Shared cross-node glossary includes: files whose fabro_* tokens describe
# the CHAIN's shared tool surface for every develop role at once, not the
# including node's own grants. Their tokens are removed from the mention
# set entirely (both arms) - counting them would fabricate HARD findings
# for tools no develop node grants (project-facts.md lists the tracker
# tool family including fabro_work_create, which belongs to the
# adjudicator/triager line, plus every other role's tools).
const GLOSSARY_INCLUDES = ['project-facts.md']

def agent-nodes [graph: string] {
    # Line-state parse of the DOT: node blocks open as `id [` (multi-line
    # or single-line), close on a bare `]`. Edge statements (`a -> b [`)
    # never match the open pattern; `//` comment lines are skipped whole
    # (they carry prose brackets like [run.model] that would corrupt a
    # naive block scan). graph/node/edge are DOT keywords, not nodes.
    let text = (open --raw $graph)
    mut blocks: list<any> = []
    mut current: string = ''
    mut body: list<string> = []
    for line in ($text | lines) {
        let trimmed = ($line | str trim)
        if ($trimmed | str starts-with '//') { continue }
        if ($current == '') {
            let open = ($trimmed | parse --regex '^(?P<id>[A-Za-z_][A-Za-z0-9_]*)\s*\[(?P<rest>.*)$')
            if ($open | is-empty) { continue }
            let head = ($open | first)
            let id = ($head | get id)
            if ($id in ['graph' 'node' 'edge']) { continue }
            let rest = ($head | get rest | str trim)
            if ($rest | str ends-with ']') {
                $blocks = ($blocks | append {id: $id, body: $rest})
            } else {
                $current = $id
                $body = []
            }
        } else {
            if ($trimmed == ']') {
                $blocks = ($blocks | append {id: $current, body: ($body | str join (char newline))})
                $current = ''
            } else {
                $body = ($body | append $trimmed)
            }
        }
    }
    # Per block: pull the prompt attribute and the grant list. A node is
    # in scope only when BOTH exist — a promptless node grants nothing to
    # compare (command stages), a prompt with no grant list has nothing to
    # violate (hello's demo node).
    $blocks | each {|b|
        let prompts = ($b.body | parse --regex 'prompt="(?P<v>[^"]*)"' | get -o v | default [])
        let grants = ($b.body | parse --regex 'x\.fabro_tools="(?P<v>[^"]*)"' | get -o v | default [])
        let prompt_val = (if ($prompts | is-empty) { null } else { $prompts | first })
        let tools = (if ($grants | is-empty) { [] } else {
            $grants | first | split row ',' | str trim | where {|t| $t != '' }
        })
        if ($prompt_val == null or ($tools | is-empty)) { [] } else {
            [{id: $b.id, prompt: $prompt_val, tools: $tools}]
        }
    } | flatten
}

# One file's fabro_* tokens with their line numbers.
def scan-text [text: string, file: string] {
    $text | lines | enumerate | each {|it|
        let hits = ($it.item | parse --regex '(?P<t>fabro_[a-z_]+)')
        if ($hits | is-empty) {
            []
        } else {
            $hits | get t | uniq | each {|t|
                {tool: $t, file: $file, line: ($it.index + 1)}
            }
        }
    } | flatten
}

# The prompt the agent sees is the composite: the prompt file plus its
# {% include %} references, recursively, cycle-safe. Glossary includes
# (the cross-node tool tables) contribute NO mentions.
def scan-with-includes [file: string, seen: list<string>] {
    if ($file in $seen) { return [] }
    let seen_next = ($seen | append $file)
    let text = (open --raw $file)
    mut found: list<any> = (scan-text $text $file)
    let includes = ($text | lines | enumerate | each {|it|
        let m = ($it.item | parse --regex '\{%\s*include\s+"(?P<f>[^"]+)"\s+%\}')
        if ($m | is-empty) {
            []
        } else {
            [{path: (($file | path dirname) | path join ($m | first | get f)), line: ($it.index + 1)}]
        }
    } | flatten)
    for inc in $includes {
        if (($inc.path | path basename) in $GLOSSARY_INCLUDES) { continue }
        if not ($inc.path | path exists) {
            print ('WARN: include target missing: ' + $inc.path + ' (referenced from ' + $file + ':' + ($inc.line | into string) + ')')
            continue
        }
        $found = ($found | append (scan-with-includes $inc.path $seen_next))
    }
    $found
}

def exempt-why [wf: string, node: string, tool: string] {
    let hit = ($HARD_EXEMPTS | where {|e| $e.graph == $wf and $e.node == $node and $e.tool == $tool })
    if ($hit | is-empty) { null } else { $hit | first | get why }
}

def main [] {
    let graphs = (glob .fabro/workflows/*/workflow.fabro | sort)
    if ($graphs | is-empty) {
        print 'lint-tool-grants: no workflow graphs found — scope broken'
        exit 2
    }
    mut hard: list<any> = []
    mut advisory: list<any> = []
    mut exempted: list<any> = []
    mut granted_nodes = 0
    for g in $graphs {
        let dir = ($g | path dirname)
        let wf = ($dir | path basename)
        for n in (agent-nodes $g) {
            $granted_nodes += 1
            let mentions = (if ($n.prompt | str starts-with '@') {
                scan-with-includes ($dir | path join ($n.prompt | str substring 1..)) []
            } else {
                scan-text $n.prompt '(inline prompt)'
            })
            let named = ($mentions | get -o tool | default [] | uniq)
            # HARD: the prompt names a tool the grant does not admit —
            # the agent discovers the denial at call time (#1303 class).
            for t in $named {
                if ($t not-in $n.tools) {
                    let why = (exempt-why $wf $n.id $t)
                    if ($why == null) {
                        let at = ($mentions | where {|m| $m.tool == $t } | first)
                        $hard = ($hard | append {
                            graph: $wf
                            node: $n.id
                            tool: $t
                            tools: $n.tools
                            at_file: $at.file
                            at_line: $at.line
                        })
                    } else {
                        $exempted = ($exempted | append {graph: $wf, node: $n.id, tool: $t, why: $why})
                    }
                }
            }
            # ADVISORY: a granted tool the prompt never names — drift, or
            # a grant serving engine bookkeeping. Never a failure.
            for t in $n.tools {
                if ($t not-in $named) {
                    $advisory = ($advisory | append {graph: $wf, node: $n.id, tool: $t})
                }
            }
        }
    }
    print ('lint-tool-grants: ' + ($graphs | length | into string) + ' workflow graphs, ' + ($granted_nodes | into string) + ' granted agent nodes')
    for e in $exempted {
        print ('exempt: ' + $e.graph + '/' + $e.node + ' names ' + $e.tool + ' — ' + $e.why)
    }
    for h in $hard {
        print ('HARD: ' + $h.graph + '/' + $h.node + ' names ' + $h.tool + ' — x.fabro_tools grants only ' + ($h.tools | str join ','))
        print ('  at ' + $h.at_file + ':' + ($h.at_line | into string))
    }
    for a in $advisory {
        print ('ADVISORY: ' + $a.graph + '/' + $a.node + ' grants ' + $a.tool + ' — never named in the prompt: drift or engine bookkeeping; keep or drop the grant deliberately')
    }
    if ($hard | is-empty) {
        print ('lint-tool-grants: green — 0 hard findings, ' + ($advisory | length | into string) + ' advisories')
    } else {
        print ('lint-tool-grants: FAILED — ' + ($hard | length | into string) + ' hard findings, ' + ($advisory | length | into string) + ' advisories')
        exit 1
    }
}
