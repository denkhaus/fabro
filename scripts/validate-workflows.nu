#!/usr/bin/env nu
# Validate workflow graphs without a Rust test-harness cold build
# (`just validate-workflows [target]`).
#
# Approach (chosen over scoping the rule out): the `fabro-validate`
# binary pre-processes the raw graph for standalone linting: `@`-file
# refs (prompt/goal, e.g. `@prompts/simplify.md`) are resolved against the
# graph file's directory BEFORE the `unresolved_file_ref` rule runs — the
# same inline strategy as the runtime pipeline. A ref whose file genuinely
# does not exist keeps its `@` value and still fails validation; scoping
# the rule out would blanket-suppress real errors. A `model_stylesheet`
# containing template syntax is blanked (the runtime renders it before the
# `stylesheet_syntax` rule runs; raw template source cannot be linted).
#
# No Rust test harness is compiled: `cargo build -p fabro-validate
# --bin fabro-validate` builds only the validator lib and its deps — not
# the CLI, the server, or dev-dependencies. Warm runs finish in seconds
# (the cold build of the small dep subset is a one-off).
#
# `target` (optional): a workflow name (`.fabro/workflows/<name>`), a
# workflow directory, a `workflow.toml` path, or a graph file path.
# Default: every `.fabro/workflows/*/workflow.toml` graph.

def main [target: string = ""] {
    let graphs = (graph_paths $target)
    if ($graphs | is-empty) {
        print -e "validate-workflows: no workflow graphs found"
        exit 2
    }

    # Prebuilt fast path (fabro-af97): the toolchain image ships
    # fabro-validate and marks itself with FABRO_VALIDATE_PREBUILT=1 —
    # skip the cargo build entirely inside run sandboxes (the cold build
    # is exactly the cost the image bake removes). Local dev checkouts
    # keep building from source, so a stale local binary can never
    # silently validate wrong rules.
    let prebuilt_ok = ((do { $env.FABRO_VALIDATE_PREBUILT? | default "" } | str trim) == "1")
    let which_ok = ((which fabro-validate | length) > 0)
    let bin = (if $prebuilt_ok and $which_ok {
        (which fabro-validate | first | get path)
    } else {
        let build = (do { cargo build --locked --quiet -p fabro-validate --bin fabro-validate } | complete)
        if ($build.exit_code != 0) {
            print -e $build.stderr
            exit $build.exit_code
        }
        ((cargo metadata --no-deps --format-version 1 | from json).target_directory
            | path join debug fabro-validate)
    })

    print $"validate-workflows: ($graphs | length) graph"
    let result = (do { ^$bin ...$graphs } | complete)
    print $result.stdout
    if ($result.exit_code != 0) {
        print -e $result.stderr
        exit $result.exit_code
    }
    # Stage-journal inspects coverage (fabro-e907) — always over ALL graphs.
    stage_journal_coverage_check
    # Loop assets are code, not just graph references: the nushell tier
    # (parse + interpolated-regex scan) runs with graph validation so a
    # broken script never ships past the host check (scripts/verify.nu
    # class, run 01M2GVW7GGGB).
    let lint = (do { ^nu scripts/lint-nu.nu } | complete)
    print $lint.stdout
    if ($lint.exit_code != 0) {
        print -e $lint.stderr
        exit $lint.exit_code
    }
    # Loop-asset literals rot silently (unresolvable seed ids, drifted
    # justfile anchors): the prompt tier lints them with the same rule
    # (run 2026-09-16 architecture pass; fabro-41de generalized).
    let plint = (do { ^nu .fabro/scripts/prompt-lint.nu } | complete)
    print $plint.stdout
    if ($plint.exit_code != 0) {
        print -e $plint.stderr
        exit $plint.exit_code
    }
    # Conductor develop-output schema probe (fabro-3196) — deterministic
    # pin of the pass-continuity teeth.
    conductor_develop_schema_probe
}

# Conductor develop-output schema probe (fabro-3196): the conductor
# develop node routes into the revise leg via a file schema
# (`.fabro/workflows/conductor/schemas/develop-output.schema.json`,
# the fabro-017f teeth pattern) whose if/then REJECTS a
# 'Develop integrated' or 'Gate stuck — revise anyway' routing unless
# context_updates.child_run_id is a non-empty string. Basis: run
# 01M2X6AT3V4CPSJC08SMCVB172 dropped the id, the revise leg failed
# pass continuity, and run 01M2X6EC95G76YFNF1YTCJZG7P went unreviewed.
# Nushell has no built-in JSON Schema validator, so this probe pins the
# contract structurally: schema shape (enum + if/then keys) and a
# simulation of the if/then semantics on the arm payloads.
def conductor_develop_schema_probe [] {
    let schema_path = '.fabro/workflows/conductor/schemas/develop-output.schema.json'
    let schema = (open $schema_path)

    # Wiring: the conductor develop node must reference the file schema.
    let graph = (open --raw .fabro/workflows/conductor/workflow.fabro)
    if not ($graph | str contains 'output_schema="@schemas/develop-output.schema.json"') {
        print -e $"validate-workflows: conductor develop node lost its file-schema wiring \(($schema_path)\)"
        exit 1
    }

    # Routing contract: the enum covers exactly the develop node's four
    # LLM-routable outgoing labels (the 'Unrouted develop outcome' soft
    # exit is engine-side and deliberately excluded).
    let expected_labels = ["Develop integrated", "Gate stuck — revise anyway", "Tracker empty", "Develop child failed"]
    let labels = ($schema | get -o properties | get -o preferred_next_label | get -o enum | default [])
    if ($labels != $expected_labels) {
        print -e $"validate-workflows: conductor develop schema enum drifted: ($labels | to json -r)"
        exit 1
    }

    # Teeth: the allOf if/then arm must require a non-empty
    # context_updates.child_run_id on the two revise-handoff labels.
    let handoff = ["Develop integrated", "Gate stuck — revise anyway"]
    let arm = ($schema | get -o allOf | default [] | where {|a|
        let if_labels = ($a | get -o if | get -o properties | get -o preferred_next_label | get -o enum | default [])
        ($if_labels | any {|l| $l in $handoff })
    })
    if ($arm | length) != 1 {
        print -e "validate-workflows: conductor develop schema lost its if/then pass-continuity arm"
        exit 1
    }
    let then = ($arm | first | get then)
    let cu_required = ($then | get -o required | default [])
    let cu = ($then | get -o properties | get -o context_updates | default {})
    let child_required = ($cu | get -o required | default [])
    let child = ($cu | get -o properties | get -o child_run_id | default {})
    if ("context_updates" not-in $cu_required) or ("child_run_id" not-in $child_required) or (($child | get -o minLength | default 0) < 1) or (($child | get -o type | default "") != "string") {
        print -e "validate-workflows: conductor develop schema if/then arm no longer requires a non-empty child_run_id"
        exit 1
    }

    # Simulate the if/then semantics on the arm payloads (rejection
    # predicate mirrors the schema exactly: handoff label AND missing
    # context_updates OR missing/zero-length child_run_id).
    def rejected [payload: record, handoff: list] {
        let label = ($payload | get -o preferred_next_label | default "")
        if ($label not-in $handoff) { false } else {
            let cu = ($payload | get -o context_updates)
            if ($cu == null) { true } else {
                (($cu | get -o child_run_id | default "") | str length) == 0
            }
        }
    }
    let cases = [
        {name: "gate-stuck WITHOUT child_run_id rejected", payload: {outcome: "succeeded", preferred_next_label: "Gate stuck — revise anyway", context_updates: {journal: {}}}, want: true}
        {name: "gate-stuck WITH child_run_id accepted", payload: {outcome: "succeeded", preferred_next_label: "Gate stuck — revise anyway", context_updates: {child_run_id: "01M2X6EC95G76YFNF1YTCJZG7P", journal: {}}}, want: false}
        {name: "merged-path WITH child_run_id accepted", payload: {outcome: "succeeded", preferred_next_label: "Develop integrated", context_updates: {child_run_id: "01M2X6EC95G76YFNF1YTCJZG7P"}}, want: false}
        {name: "merged-path WITHOUT child_run_id rejected", payload: {outcome: "succeeded", preferred_next_label: "Develop integrated"}, want: true}
        {name: "empty child_run_id rejected (minLength)", payload: {outcome: "succeeded", preferred_next_label: "Gate stuck — revise anyway", context_updates: {child_run_id: ""}}, want: true}
        {name: "tracker-empty needs NO child_run_id", payload: {outcome: "succeeded", preferred_next_label: "Tracker empty"}, want: false}
    ]
    for $case in $cases {
        let got = (rejected $case.payload $handoff)
        if $got != $case.want {
            print -e $"validate-workflows: conductor develop schema probe FAILED: ($case.name) \(got rejected=($got), want=($case.want))"
            exit 1
        }
    }
    print $"validate-workflows: conductor develop schema probe ok \(($cases | length) cases\)"
}

# Class-level inspects-coverage lint (fabro-e907): every workflow whose
# workflow.toml carries the stage-journal hook must appear in some
# graph's inspects list — else a journal-producing run has no consumer
# and its journals are silently lost (fabro-905d blind-spot class).
# Always runs over ALL graphs, never just a named target.
def stage_journal_coverage_check [] {
    let journal_workflows = (glob .fabro/workflows/*/workflow.toml | each {|m|
        let hooks = (open $m | get -o run | get -o hooks | default [])
        if ($hooks | any {|h| ($h | get -o name | default "") == "stage-journal" }) {
            $m | path dirname | path basename
        }
    } | compact)
    let covered = (glob .fabro/workflows/*/workflow.fabro | each {|g|
        open --raw $g | parse --regex 'inspects\s*=\s*"(?<slugs>[^"]*)"'
    } | flatten | get -o slugs | default [] | each {|row| $row | split row "," } | flatten
        | each {|s| $s | str trim } | where {|s| not ($s | is-empty) } | uniq)
    let uncovered = ($journal_workflows | where {|w| $w not-in $covered })
    if not ($uncovered | is-empty) {
        print -e $"validate-workflows: stage-journal workflows missing from every inspects list: ($uncovered | str join ', ')"
        exit 1
    }
    print $"validate-workflows: inspects coverage ok \(($journal_workflows | length) stage-journal workflow\)"
}

# Resolve the target (or all workflows) to a list of graph file paths.
def graph_paths [target: string] {
    if ($target | is-empty) {
        glob .fabro/workflows/*/workflow.toml | each { graph_from_toml $in }
    } else if ($target | str ends-with ".fabro") {
        [$target]
    } else if ($target | str ends-with ".toml") {
        [($target | path expand)] | each { graph_from_toml $in }
    } else {
        let dir = (if ($target | str starts-with ".") { $target } else { $".fabro/workflows/($target)" })
        let toml = ($dir | path join workflow.toml)
        if not ($toml | path exists) {
            print -e $"validate-workflows: no workflow at ($toml)"
            exit 2
        }
        [(graph_from_toml $toml)]
    }
}

# The graph path named by a workflow.toml (default: workflow.fabro),
# resolved relative to the workflow directory.
def graph_from_toml [manifest: path] {
    let settings = (open $manifest)
    let graph = ($settings | get -o workflow | get -o graph | default "workflow.fabro")
    ($manifest | path dirname | path join $graph)
}
