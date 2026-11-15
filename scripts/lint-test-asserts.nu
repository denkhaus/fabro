#!/usr/bin/env nu
# Lint test assertions: HTTP-status asserts in axum tests SHOULD go
# through the shared fabro_test helpers (expect_axum_*/assert_axum_*),
# never bare `assert_eq!(... .status() ...)` — the testing strategy's
# rule. The sprint-89 review caught two new suites written the bare
# way under context pressure; this check makes the rule visible.
#
# MODE: report by default (the bare style is the repo-wide STATUS QUO —
# ~130 pre-existing sites across landed code; the strategy rule is
# aspirational until a migration decision item lands). Pass
# --gate for the red-door form (exit 1 on any hit) — the shape a
# migration lands on.

def main [--gate]: nothing -> nothing {
    let pattern = 'assert_eq!\([^;\n]{0,160}\.status\(\)'
    let res = (do { rg -n --no-heading $pattern lib apps } | complete)
    if $res.exit_code == 2 {
        print $"lint-test-asserts: rg failed: ($res.stderr | str substring 0..200)"
        exit 2
    }
    let hits = ($res.stdout | lines | where {|l| $l | is-not-empty })
    if ($hits | is-empty) {
        print 'lint-test-asserts: green — no bare status assertions'
        return
    }
    let by_area = ($hits
        | parse --regex '^(?P<path>[^:]+):'
        | group-by path
        | items {|path, rows| {path: $path, count: ($rows | length)} }
        | sort-by count --reverse)
    print $"lint-test-asserts: ($hits | length) bare status assertions across ($by_area | length) files — top areas:"
    for row in ($by_area | first 8) {
        print $"  ($row.count)  ($row.path)"
    }
    if $gate {
        print 'lint-test-asserts: GATE — bare status assertions found (use the fabro_test helpers)'
        exit 1
    }
    print 'lint-test-asserts: report mode (the status quo is bare; --gate reddens once the migration decision lands)'
}
