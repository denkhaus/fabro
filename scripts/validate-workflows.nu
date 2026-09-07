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

    let build = (do { cargo build --locked --quiet -p fabro-validate --bin fabro-validate } | complete)
    if ($build.exit_code != 0) {
        print -e $build.stderr
        exit $build.exit_code
    }
    let bin = ((cargo metadata --no-deps --format-version 1 | from json).target_directory
        | path join debug fabro-validate)

    print $"validate-workflows: ($graphs | length) graph"
    let result = (do { ^$bin ...$graphs } | complete)
    print $result.stdout
    if ($result.exit_code != 0) {
        print -e $result.stderr
        exit $result.exit_code
    }
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
