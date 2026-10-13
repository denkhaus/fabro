#!/usr/bin/env nu
# Provider-window gate twin-smoke PRESENCE battery (fabro-2919): the twin
# scenarios themselves are a Rust test (they must run in every plain
# `cargo nextest run -p fabro-server` round — the loop lane never compiles
# product code, so a cargo round does not belong in this registry); this
# battery is the silent-deletion net for that pin, the fabro-8b38 class:
# the fork-only test file, both scenario names, its TOUCHPOINTS row, the
# justfile ceremony recipe, and the fabro-test sticky scenario flag it
# depends on must all keep existing. A renamed or dropped pin REDs here
# instead of green-lining the next gate round.
# prompt-lint: synthetic-seed-ids
# ^ the crate PATH checked below (lib/foundation/fabro-test/src/lib.rs)
# carries a segment matching the seed-id shape; it is a path, not a seed.

const ROOT = (path self | path dirname | path dirname | path dirname)

def fail [what: string]: nothing -> nothing {
    print -e $"window-gate-smoke: FAIL — ($what)"
    exit 1
}

def must_contain [path: string, needle: string, what: string]: nothing -> nothing {
    let full = ($ROOT | path join $path)
    if not ($full | path exists) {
        fail $"missing ($path)"
    }
    let text = (open --raw $full)
    if not ($text | str contains $needle) {
        fail $"($what): ($path) no longer contains '($needle)'"
    }
}

# 1. The fork-only twin test file exists and carries both scenarios.
must_contain 'lib/apps/fabro-server/src/server/fork_window_gate_twin_tests.rs' 'a_scripted_usage_window_holds_the_tick_and_refuses_manual_fires' 'the hold + manual-refusal scenario pin'
must_contain 'lib/apps/fabro-server/src/server/fork_window_gate_twin_tests.rs' 'a_window_reopen_fires_on_the_next_recheck_tick_and_clears_the_facts' 'the reopen-fires scenario pin (the announce-style forever-hold net)'

# 2. The module is wired into the server (a dropped mod declaration means
#    the file never compiles — the pin silently stops running).
must_contain 'lib/apps/fabro-server/src/server.rs' 'mod fork_window_gate_twin_tests;' 'the twin-smoke module wiring'

# 3. The two-pin rule: the pin file is named in the touchpoints registry.
must_contain 'TOUCHPOINTS.md' 'fork_window_gate_twin_tests.rs' 'the TOUCHPOINTS row'

# 4. The deploy ceremony recipe names the smoke command.
must_contain 'justfile' 'smoke-window-gate' 'the justfile ceremony recipe'
must_contain 'justfile' 'cargo nextest run -p fabro-server window_gate' 'the recipe runs the window_gate filter'

# 5. The twin scenario the pin depends on stays sticky-capable (a client
#    that retries a scripted 429 must keep meeting the scripted answer).
must_contain 'lib/foundation/fabro-test/src/lib.rs' 'pub fn sticky' \
    'TwinScenario::sticky'

print 'window-gate-smoke: presence battery green'
