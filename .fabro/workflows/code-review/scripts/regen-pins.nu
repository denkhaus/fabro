#!/usr/bin/env nu
# Regen the code-review workflow's graph pin manifest (pins.json).
#
# The prepare node's preamble verifies every pins.json entry against the
# working tree before the engine starts (runtime integrity wall; any drift
# fails the run). This script is the generator side: it derives the pin set
# from the workflow's runtime-control globs and rewrites pins.json
# deterministically (entries sorted by path). Run it after any edit to a
# covered file and commit the manifest together with that edit.
#
# Usage:
#   nu .fabro/workflows/code-review/scripts/regen-pins.nu          # regenerate
#   nu .fabro/workflows/code-review/scripts/regen-pins.nu --check  # verify only
#
# The pin set mirrors what a review run executes or loads: engine scripts,
# the report spec and template, schemas, prompts, and the built-in rule
# manifest (the builtin YAML rules are pinned transitively through their
# own manifest). pins.json, the graph, workflow.toml, requirements-rules,
# and this generator are not pinned: the admitted graph is the trust
# anchor and a manifest cannot pin itself.

const WF = ".fabro/workflows/code-review"

def pinned-files [root: path] {
    (glob $"($root)/scripts/*.py")
    | append (glob $"($root)/specs/*")
    | append (glob $"($root)/templates/*")
    | append (glob $"($root)/schemas/*")
    | append (glob $"($root)/prompts/**/*.j2")
    | append [$"($root)/rules/builtin-manifest.json"]
    | each {|f| $f | path relative-to $root }
    | uniq
    | sort
}

def build-manifest [root: path] {
    {
        files: (pinned-files $root | each {|rel|
            {
                path: $rel
                sha256: (open --raw $"($root)/($rel)" | hash sha256)
            }
        })
        version: 1
    }
}

def main [--check] {
    let root = ($WF | path expand)
    let manifest = (build-manifest $root)
    let rendered = ((build-manifest $root | to json --indent 2) + "\n")
    let count = ($manifest | get files | length)
    if $check {
        let current = (open --raw $"($root)/pins.json")
        if $current == $rendered {
            print $"pins.json: green ($count) pinned files"
        } else {
            print "pins.json is stale — regenerate with: nu .fabro/workflows/code-review/scripts/regen-pins.nu"
            exit 1
        }
    } else {
        $rendered | save --force $"($root)/pins.json"
        print $"pins.json: wrote ($count) pinned files"
    }
}
