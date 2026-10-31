#!/usr/bin/env nu
# Tool-pin parity battery (fabro-1893, arch-gate sprint-39 aftermath): the
# seeds CLI version is pinned in TWO places that must never drift apart —
# mise (.mise.toml, dev hosts + run sandboxes via mise) and the toolchain
# image (.fabro/Dockerfile.toolchain, the /usr/local/bin/seeds binary the
# image bakes). A drift means run sandboxes and dev hosts speak different
# CLI dialects (the 2026-10-14 history: mise 0.8.0 vs image 0.7.0 vs a
# stale deployed image at 0.5.0). This battery REDs on any divergence.

# Battery contract: pure-logic, no cargo, no live server (battery-runner tier).
def read-mise-seeds []: nothing -> string {
    open .mise.toml | get tools | get 'cargo:seeds'
}

def read-dockerfile-seeds []: nothing -> string {
    let line = (open --raw .fabro/Dockerfile.toolchain | lines | where $it =~ 'ARG SEEDS_VERSION' | first)
    $line | parse --regex 'ARG SEEDS_VERSION=(?<v>[0-9]+\.[0-9]+\.[0-9]+)' | get 0.v
}

# The mulch pin (work#1070, the ADR-0023 cutover): .mise.toml's
# cargo:mulch must equal the cargo:mulch@X in the toolchain image's
# mise toolset block — the same drift class as seeds, one guard.
def read-mise-mulch []: nothing -> string {
    open .mise.toml | get tools | get 'cargo:mulch'
}

def read-dockerfile-mulch []: nothing -> string {
    let line = (open --raw .fabro/Dockerfile.toolchain | lines | where $it =~ 'cargo:mulch@' | first)
    $line | parse --regex 'cargo:mulch@(?<v>[0-9]+\.[0-9]+\.[0-9]+)' | get 0.v
}

def fail [msg: string]: nothing -> nothing {
    print -e $"tool-pin-parity: ($msg)"
    exit 1
}

def main []: nothing -> nothing {
    if (which mise | is-empty) { fail 'mise absent — cannot read the tool pins' }
    let mise_v = (read-mise-seeds)
    let df_v = (read-dockerfile-seeds)
    if ($mise_v | is-empty) { fail '.mise.toml carries no cargo:seeds pin' }
    if ($df_v | is-empty) { fail 'Dockerfile.toolchain carries no ARG SEEDS_VERSION' }
    if $mise_v != $df_v {
        fail $"seeds pin drift: .mise.toml ($mise_v) != Dockerfile.toolchain ($df_v) — repin BOTH in one change, see seed fabro-1893"
    }
    let mise_mulch = (read-mise-mulch)
    let df_mulch = (read-dockerfile-mulch)
    if ($mise_mulch | is-empty) { fail '.mise.toml carries no cargo:mulch pin' }
    if ($df_mulch | is-empty) { fail 'Dockerfile.toolchain carries no cargo:mulch pin in the mise toolset block' }
    if $mise_mulch != $df_mulch {
        fail $"mulch pin drift: .mise.toml ($mise_mulch) != Dockerfile.toolchain ($df_mulch) — repin BOTH in one change, denkhaus/work#1070"
    }
    print $"tool-pin-parity: ok — seeds ($mise_v), mulch ($mise_mulch) aligned in mise and toolchain image"
}
