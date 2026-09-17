# Upstream offer: tolerate real legacy run-history rows on activation

This branch (from denkhaus/fabro) offers two small, independent fixes that
both make run-history activation survive rows real production stores
actually contain. Both are landed and battle-tested in the denkhaus fork;
this branch ports them onto current upstream `main`.

## Case 1: 5-segment legacy run-catalog keys

**Problem.** `parse_run_catalog_key` in
`lib/components/fabro-store/src/keys.rs` only accepted the 4-segment
canonical marker `runs/_index/by-start/<run_id>`. The catalog writer that
populated real deployments prefixed each run id with its start date,
producing `runs/_index/by-start/<YYYY-MM-DD>/<run_id>`. Activation rejected
those markers.

**Fix.** The parser accepts both layouts; the 5-segment form requires the
middle segment to be a valid `YYYY-MM-DD` date (validated with
`chrono::NaiveDate::parse_from_str`), anything else is still rejected. The
key-builder helpers are gated `#[cfg(any(test, feature = "test-support"))]`.

**Tests.** `run_catalog_keys_parse_canonical_and_legacy_layouts` (both
layouts parse) and `run_catalog_keys_reject_non_date_middle_segments`
(`not-a-date` middle segment, bare `runs`, and truncated prefixes reject).

## Case 2: variantless legacy event names

**Problem.** The sandbox-driver adoption removed the `EventBody` variants
for `sandbox.git.started/completed/failed` and
`sandbox.cleanup.started/completed/failed`, but the names remained in
`is_known_event_name`. Run-history activation replays every stored event,
so a single pre-adoption `sandbox.git.*` row hard-failed startup — a
crash-loop on real production data.

**Fix.** `RunEvent::from_parts` in
`lib/foundation/fabro-types/src/run_event/mod.rs` degrades exactly those
six known-but-variantless names to `EventBody::Unknown` (preserving name
and properties) instead of returning `Err`. Unknown event names were
already tolerated; this only stops known-but-retired names from aborting
activation.

**Tests.** `legacy_variantless_event_names_read_back_as_unknown` feeds all
six names through `RunEvent::from_json_str` and asserts an `Unknown` body.

## Test evidence

Both crates pass on this branch:

```
cargo nextest run -p fabro-types -p fabro-store
```

including the four new/ported tests named above. The port mirrors the
fork's implementation (fallback order and gating) with no behavioral
deviation.
