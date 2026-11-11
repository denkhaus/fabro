# ADR-0043: Retire the TOUCHPOINTS two-pin rule

- Status: accepted
- Date: 2026-11-11
- Supersedes: the two-pin rule introduced with the touchpoints registry
  (fabro-3e3c) and machine-checked since fabro-de32
- Work item: denkhaus/work#1273 (user decision 2026-11-11, grilled)

## Context

The touchpoints registry (`TOUCHPOINTS.md` at the repo root) and the
two-pin rule — every durable fork feature keeps a registry row AND a
fork-only test file, enforced by the touchpoints-parity battery — were
built when upstream merges could silently drop fork features.

That threat died with ADR-0024: the upstream intake is frozen, the flow
is outbound-only (we offer selected fixes upstream; nothing merges back).
The rule's reason is gone, but its cost stayed: every fork feature pays
a registry row, the battery gates every loop pass, and the sprint close
runs the parity check.

One real threat survives the freeze, narrower than the old one: a
`just vendor import <name>` from a wrong fork state rewrites
`vendor/PINS.toml` cleanly, so the content-hash check passes against
the new state while the tree silently lost a fork feature. The
sandbox-driver fork birth (work#1253, same day) walked exactly this
path and pinned it with a vendor-seam presence test.

## Decision

1. The registry, the parity battery, and the two-pin RULE are RETIRED:
   `TOUCHPOINTS.md` deleted, `touchpoints-parity-fixtures.nu` deleted,
   the battery row and the sprint-close call removed.
2. The `lib/**/tests/fork_*.rs` files STAY, as ordinary regression
   tests. Most carry the only behavior coverage their features have
   (taxonomy serialization, projection folds, tool wire shapes);
   deleting them was considered and rejected in the grill.
3. Presence pins remain in force ONLY at vendor seams: where a feature
   lives inside `vendor/petri`, `vendor/pebble`, or
   `vendor/sandbox-driver`, a fork-only presence test may assert the
   feature survives the vendor seam (drift guard against wrong-state
   imports). No registry, no per-feature duty, no battery.

## Consequences

- Fork features no longer require bookkeeping; the cost of adding one
  drops to the test itself.
- The vendor-import seam is guarded per-feature where an author judges
  it worthwhile (for example `fork_build_diagnostics.rs`), not by a
  mandatory pair.
- The sprint close no longer runs parity; the battery list in
  `scripts/qualitygate.nu` shortens accordingly.
- Historical investigation of fork features uses git history and the
  test files themselves, not the registry.
