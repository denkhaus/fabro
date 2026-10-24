#!/usr/bin/env nu
# Dependency freeze pin (ADR-0027, 2026-10-09): this line does NOT track
# upstream of its engine dependencies. Cargo.lock pins them to exact revs;
# this battery asserts those revs against the recorded list below, so a
# bump cannot happen silently — a dep bump is a deliberate edit HERE plus a
# recorded reason (ADR-0027 decision 2/3).
#
# Both directions are checked (the two-pin shape used across this repo): a
# locked engine package missing from PINS REDs, and a PINS entry that the
# lock no longer carries REDs.
#
# Bump 2026-10-24 (pebble 59f5699de785 -> 47a036fd6004, fabro-c98c):
# the request-path tool-error wrap now REPLACES the structured flag with
# its "[TOOL ERROR] " content marker. The chat-completions wire (zai's
# profile) has no error marker and reported unsupported_control for every
# request that still carried the flag — the production WARN storm on zai
# loop runs after fork.24 (4x per planner session while the content wrap
# already worked). A result whose marker never reaches the content keeps
# the flag, so an invisible failure still reports. Patching our forks is
# expected; the rev move is recorded here per ADR-0027 decision 2/3.
# Bump 2026-10-23 (petri 13e4e6627693 -> 2dedbc295a1f + pebble
# 5897c0635698 -> 59f5699de785, fabro-d4f2 + fabro-c98c, sprint 50 part 2):
# petri — a plain interrupt settles the attempt after a 60s grace with the
# terminal class "interrupted" (InterruptGrace capability overrides; both
# backends), so an interrupted turn can no longer wedge a run in running
# forever (incident 01M59MG9XS2A). pebble — the openai-wire [TOOL ERROR]
# wrap rides the model service's request path, covering mid-session tool
# failures (the mimo class the build-time render missed; zai runs the
# OpenAi profile kind). Both user-grilled 2026-10-20 (d4f2 design GO).
# Patching our forks is expected; the rev move is recorded here per
# ADR-0027 decision 2/3.
# Bump 2026-10-17b (petri 78c524bda565 -> 13e4e6627693): retryability by
# failure kind (fabro-95d4, user decision) — retry_requested only for
# rate_limit/network/timeout (same context re-sendable); 5xx keeps
# llm:server (the request was processed and failed — not repeatable with
# the same context), quota keeps its class. The 8fcb471 drift test is
# green as originally written. Patching our forks is expected; the rev
# move is recorded here per ADR-0027 decision 2/3.
# Bump 2026-10-17 (petri 323b8aaaef56 -> 78c524bda565): per-hook declared
# secrets (fabro-225f) — a [[run.hooks]] host command hook declares
# `secrets = [...]`, the engine resolves the names against the run's
# SecretProvider and injects them into exactly that hook's process env
# (provider-agnostic; no env allowlisting, no credential file). Includes a
# tip-hygiene commit (pebble.rs fmt + clippy) that brought `mise run lint`
# back to green on the fork tip. Patching our forks is expected; the rev
# move is recorded here per ADR-0027 decision 2/3.
# Bump 2026-10-12 (petri 8fcb47197ac1 -> 323b8aaaef56): Fabro stream volume —
# the pebble delta mirror coalesces (fabro-9dff: 43,915 token-delta events /
# 46.8 MB in one battery run starved the store read pool). Only petri moved;
# sandbox-driver deliberately restored to its frozen rev (cargo's branch-dep
# re-resolution dragged it to head; lock blocks hand-restored).
# Bump 2026-10-11 (petri b03b0c3a5079 -> 323b8aaaef56): Fabro's retry fix —
# a retryable native (Pebble) LLM failure now rides `retry_requested`, the
# one class every retry policy retries (run 01M4NSBKV6ZBPAFFEFND65HEP9: a
# zai 429 killed the loop planner with retries: 0). Patching our forks is
# expected; the rev move is recorded here per ADR-0027 decision 2/3.
#
# Scope: the GIT-pinned engine dependencies only (lithos-llm,
# sandbox-driver, twins, daytona-sdk-rust). petri and pebble are VENDORED
# in-tree since fabro-eeee (2026-10-24): their pin is the vendor snapshot
# in vendor/PINS.toml, checked by `scripts/vendor-pins-fixtures.nu` (a path package
# leaves no rev in the lock, so this battery cannot see them). Our own
# seeds crate is tag-pinned.

const LOCK = 'Cargo.lock'

const ENGINE_REPOS = ['lithos-llm', 'sandbox-driver', 'twins', 'daytona-sdk-rust']

# package name -> allowed rev(s), 12-hex prefixes.
const PINS = {
    "daytona-api-client": ["0e69058c888a"]
    "daytona-sdk": ["0e69058c888a"]
    "daytona-toolbox-client": ["0e69058c888a"]
    "lithos-llm": ["fd42e6b2805b"]
    # petri + pebble are VENDORED since fabro-eeee (2026-10-24): their
    # provenance lives in vendor/PINS.toml, checked by scripts/vendor-pins-fixtures.nu.
    "sandbox-driver": ["90b0d8257671"]
    "sandbox-driver-daytona": ["236196ed134d"]
    "sandbox-driver-daytona-config": ["90b0d8257671"]
    "sandbox-driver-docker": ["236196ed134d"]
    "sandbox-driver-docker-config": ["90b0d8257671"]
    "sandbox-driver-host": ["90b0d8257671"]
    "sandbox-driver-protocol": ["90b0d8257671"]
    "sandbox-driver-testing": ["90b0d8257671"]
    "twin-core": ["19bf6ae2fbfc"]
    "twin-openai": ["19bf6ae2fbfc"]
}

# Every engine package the lock carries: name -> rev12.
def locked-revs [] {
    mut pkgs = {}
    mut name = ''
    mut source = ''
    mut lines = (open --raw $LOCK | lines)
    $lines = ($lines | append '')
    for line in $lines {
        if ($line | str starts-with 'name = ') {
            $name = ($line | str replace 'name = ' '' | str trim | str replace --all '"' '')
            $source = ''
        } else if ($line | str starts-with 'source = "git+') {
            $source = $line
        } else if ($line | str trim | is-empty) {
            if ($name | is-not-empty) and ($source | is-not-empty) {
                let repo = ($source | str replace 'source = "git+https://github.com/' '' | split row '?' | first)
                let owner_repo = ($repo | split row '/' | last)
                if ($ENGINE_REPOS | any {|r| $owner_repo == $r }) {
                    let rev = ($source | split row '#' | last | str replace '"' '' | str substring 0..11)
                    $pkgs = ($pkgs | insert $name $rev)
                }
            }
            $name = ''
            $source = ''
        }
    }
    $pkgs
}

def main [] {
    let locked = (locked-revs)
    mut red = false

    # Forward: every locked engine package must be pinned here at that rev.
    for entry in ($locked | transpose name rev) {
        let name = $entry.name
        let rev = $entry.rev
        if not ($name in $PINS) {
            print $"RED: ($name) is an engine dependency but not pinned in this battery — add it with a reason (ADR-0027)"
            $red = true
        } else if not ($rev in ($PINS | get $name)) {
            print $"RED: ($name) moved to ($rev) — PINS records ($PINS | get $name | str join ', '); a dep bump is a deliberate decision per ADR-0027: reason + edit this list"
            $red = true
        }
    }
    if not $red {
        print $"ok: ($locked | transpose | length) locked engine packages match their recorded revs"
    }

    # Reverse: a pinned package the lock no longer carries is a stale pin.
    for entry in ($PINS | transpose name revs) {
        if not ($entry.name in $locked) {
            print $"RED: PINS records ($entry.name) but the lock no longer carries it — drop the stale pin"
            $red = true
        }
    }
    if not $red {
        print "ok: every recorded pin still exists in the lock"
    }

    # Teeth: the parser must actually see revs (a broken parse would make
    # both loops above trivially green). 14 git-pinned engine packages
    # remain after the petri/pebble vendoring (fabro-eeee); the vendored
    # 18 are checked by scripts/vendor-pins-fixtures.nu.
    if ($locked | transpose | length) < 10 {
        print $"RED: the parser found only ($locked | transpose | length) engine packages — far too few; the lock parse is broken, this battery proves nothing"
        $red = true
    } else {
        print $"ok: the lock parse sees the expected engine mass \(($locked | transpose | length) packages)"
    }

    if $red { exit 1 }
    print "dep-pins-fixtures: green"
}
