#!/usr/bin/env nu
# The terminal-run-status vocabulary (denkhaus/work#1446, arch candidate
# C2): "Is this run finished?" is GLOSSARY domain vocabulary, but six
# consumers each spelled their own list — and the lists disagreed
# (push-gate knew no human_needed and no dead; stall-check and the shim
# knew no canceled spellings; salvage-pull knew three kinds;
# run_workflow compared against one). One predicate, one list, consumed
# everywhere.
#
# THE KIND SET: the engine's wire vocabulary is fabro-types
# `RunStatusKind` (snake_case serde) — its terminal family is exactly
# succeeded / failed / human_needed / dead (`RunStatus::is_terminal`,
# work#1072: the decision request is TERMINAL, never a failure or a
# block). ADR-0045 (parked runs are waiting states) SUPERSEDES that
# terminality clause: under its P2, `human_needed` leaves
# `is_terminal` and grows the waiting family. The interim decided
# semantics below stand until P2 lands; when ADR-0045's P3 consumer
# split opens, it lands INSIDE this module — shrink TERMINAL_KINDS,
# add the waiting family, and give each consumer the predicate its
# line names (per-consumer imports: push-gate exempts waiting,
# watchdog counts it neither-completed-nor-stalled, salvage refuses).
#
# A CANCELLED run is a different thing: petri's engine-level
# `RunStatus::Cancelled` maps to platform `Failed { reason:
# FailureReason::Cancelled }` — NEITHER canceled spelling is ever
# emitted as a status kind. Both spellings stay ACCEPTED here because
# the platform's own FromStr accepts both ("canceled" | "cancelled")
# and the retired consumers spelled them; the predicate never narrows a
# spelling a consumer used to accept.
#
# Decided semantics (Jev pre-screen 2026-11-18, terminal_not_blocking,
# work#1446): human_needed counts as terminal EVERYWHERE, including the
# push gate — a parked run never blocks pushes by itself; an
# actually-held claim still blocks through the claim arm (the
# reconciler releases only a refused verdict, work#1072/#1419). The
# claim arm stays the sole holder of "work is genuinely in flight".
#
# Pins: .fabro/scripts/lane-commons-fixtures.nu (families 7a-7c) and
# the watchdog stall smoke's graph arm (work#1168 fold).

# The terminal family: the four kinds the engine emits, then the two
# canceled spellings it accepts but never emits. A platform-side rename
# of a kind lands as an unknown string — never terminal — so every
# consumer fails CLOSED and this battery reds (the rename cannot ride
# in silently).
export const TERMINAL_KINDS = [succeeded failed human_needed dead canceled cancelled]

# The one green kind (the integration verdict: run_workflow integrates
# nothing but a succeeded run).
export const SUCCESS_KIND = 'succeeded'

# Is this status kind a terminal one? Unknown kinds are NOT terminal —
# every consumer's posture on an unknown kind is the conservative one
# (the push gate classifies it active and refuses; the salvage tools
# refuse the run; the watchdog counts no completed pass).
export def is-terminal-kind [kind: string]: nothing -> bool {
    $kind in $TERMINAL_KINDS
}

# Is this status kind THE green one? (run_workflow's integration gate.)
export def is-success-kind [kind: string]: nothing -> bool {
    $kind == $SUCCESS_KIND
}

# The decision-request kind (work#1072): the run itself asks for a human —
# the adjudicator fast-path parks such runs LLM-free. One spelled home
# beside the other single-kind predicate so no consumer re-types the
# literal (the work#1446 same-file adoption).
export const HUMAN_NEEDED_KIND = 'human_needed'

export def is-human-needed-kind [kind: string]: nothing -> bool {
    $kind == $HUMAN_NEEDED_KIND
}

# The push gate's blocking classification, PURE over captured door JSON
# (work#1446) — no server in reach:
#   runs   = `fabro ps --all --json` rows (the FULL listing: plain ps
#            scopes to the server's own is_active set, which hides a
#            parked human_needed run — the gate classifies kinds itself
#            and must resolve parked runs for the claim arm),
#   claims = the project claims view rows (camelCase runId).
# Returns {active: [ps rows still working], blocking: [claims that hold
# the line]}.
#
# Run arm: a run is active unless its kind is terminal (unknown kinds
# are active — fail closed).
#
# Claim arm: a claim blocks while its work is genuinely in flight —
#   - live states only (granted / active / awaiting_merge),
#   - a claim with NO run id fails CLOSED (blocks),
#   - a claim on an ACTIVE run blocks,
#   - a claim on a PARKED (human_needed) run blocks — the claim holds
#     until the adjudicator's verdict and the reconciler releases only
#     a refused verdict (work#1072/#1419),
#   - a claim on another terminal run (succeeded / failed / dead) does
#     NOT block: the reconciler heals it (run status is the truth; the
#     2026-11-13 incident: a failed run's orphan claim held the trunk
#     for an hour with nothing in flight),
#   - a claim whose run id resolves to NO row of the full listing
#     (deleted / archived run) stays reconciler territory.
export def gate-classification [runs: list<any>, claims: list<any>]: nothing -> record {
    let active = ($runs | where {|r|
        (is-terminal-kind ($r | get -o status | get -o kind | default 'unknown')) == false
    })
    # The 26-char prefix table (run ids are ULIDs; both doors prefix-
    # compare — the retired consumer behavior, pinned in the fixtures).
    let kinds = ($runs | each {|r|
        {
            id: ($r | get -o run_id | default '' | str substring 0..26),
            kind: ($r | get -o status | get -o kind | default 'unknown'),
        }
    })
    let blocking = ($claims | where {|c|
        let state = ($c | get -o state | default '')
        if $state not-in ['granted' 'active' 'awaiting_merge'] { return false }
        let claim_run = ($c | get -o runId | default '' | str substring 0..26)
        if ($claim_run | is-empty) { return true }
        let row = ($kinds | where {|k| $k.id == $claim_run } | get -o 0 | default null)
        if $row == null { return false }
        if ($row.kind == 'human_needed') { return true }
        (is-terminal-kind $row.kind) == false
    })
    {active: $active, blocking: $blocking}
}
