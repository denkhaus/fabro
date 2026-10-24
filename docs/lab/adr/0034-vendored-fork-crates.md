# ADR-0034: Vendored fork crates — petri and pebble live in-tree as snapshots

- Status: Accepted (2026-10-24)
- Deciders: user (decision: pull the vendoring forward, no run-based work
  possible while the provider quota is closed), agent (design grill-lite
  2026-10-23, implementation fabro-eeee)
- Evidence: `vendor/PINS.toml` (provenance of both trees),
  `scripts/vendor-pins-fixtures.nu` (the battery), `scripts/vendor-sync.nu` (import /
  export / status), the path `[patch]` blocks in `Cargo.toml`
- Relates to: ADR-0027 (dep-fork freeze — unchanged by this ADR),
  ADR-0024 (deliberate intake), ADR-0022 (one change unit)

## Context

The line depends on two forks of upstream engine repositories:
`denkhaus/petri` (15 crates) and `denkhaus/pebble` (3 crates). Both are
consumed through `[patch]` blocks that point at the fork's `fabro` branch,
with the exact rev pinned in `Cargo.lock` and asserted by
`.fabro/scripts/dep-pins-fixtures.nu` (ADR-0027 point 3).

That arrangement carried four recurring costs:

1. **A fork fix is a three-repository dance.** Edit in `~/dev/petri`, push,
   move the lock rev by hand (`cargo update -p` re-resolves unrelated git
   deps, so the reliable move is a `sed` on the lock's `source` line), bump
   the rev list in `dep-pins-fixtures.nu`, then rebuild. Every engine fix
   paid it.
2. **The engine source is not where the consuming code is.** An agent
   reading `fabro-petri` cannot follow a call into petri without a second
   checkout; a review of a fork fix cannot see the consuming side.
3. **Two large checkouts on every machine.** `~/dev/petri` (~64 GB) and
   `~/dev/pebble` (~41 GB) were prerequisites for reading the engine, for
   the toolchain image cook (the cook context needed the fork sources), and
   for `TOUCHPOINTS.md` test commands.
4. **The image cook needed network access to GitHub** for the fork repos,
   and a fork-branch move could change what the cook resolved without any
   change in this repository.

ADR-0027 froze upstream intake; it did not address where the frozen source
lives. The fork repositories also stay the history archive and the
upstream-PR vehicle — deleting history is not the goal.

## Decision

1. **The fork crates are vendored in-tree**: `vendor/petri` and
   `vendor/pebble` are snapshots (no fork history) of the fork branches'
   trees. The `[patch]` blocks in `Cargo.toml` resolve them by `path`.
2. **The vendored workspaces stay their own workspaces.**
   `Cargo.toml` lists `exclude = ["vendor/petri", "vendor/pebble"]`, so
   their feature graphs and their lockfiles never unify with fabro's; their
   suites run in their own workspace (`cd vendor/petri && cargo nextest run
   --locked …`). `--locked` on purpose: a run that rewrites the vendored
   lockfile is a content drift, not a test run.
3. **Provenance lives in `vendor/PINS.toml`**, one section per fork:
   `fork_sha`, `branch`, `upstream`, `upstream_sha` (the validated base),
   `tree_sha` (the fork commit's git tree), `content_sha256` (recomputed
   from the tree), `files`, `crates` (derived from the `[patch]` block —
   the crates fabro actually consumes) and `imported`.
   `scripts/vendor-pins-fixtures.nu` (`just vendor-pins`) verifies all of it: the
   content hash against the tree, the tree sha against the fork commit when
   a fork clone is reachable (fail-open with a note otherwise), and that
   every patched crate appears in `Cargo.lock` **without** a git source — a
   vendored crate that silently resolves from git again is the failure this
   catches.
4. **Sync is a two-command surface** over one managed clone per fork at
   `tmp/vendor/<name>` (gitignored, created on demand):
   `just vendor import <name>` (fork tip → tree + PINS) and
   `just vendor export <name> --message '…'` (tree → one fork commit,
   pushed). `just vendor status` reports drift without cloning.
5. **The fork repositories stay authoritative for history and for
   upstream.** A fork fix made in-tree is exported as a fork commit; an
   upstream contribution still starts from the fork branch. The fabro
   repository carries no fork history on purpose.
6. **The pin discipline of ADR-0027 point 3 is unchanged in force, changed
   in mechanism.** A vendor bump is still a deliberate, recorded move: one
   commit carrying the new tree, the new `PINS.toml` section and (if the
   dependency graph moved) the new `Cargo.lock`. The vendored crates leave
   `dep-pins-fixtures.nu`'s rev list — a path package carries no rev in the
   lock — and are covered by `vendor-pins-fixtures.nu` instead. The battery's teeth
   threshold drops from 20 to 10 git-pinned engine packages accordingly.
7. **The toolchain image cook builds from the tree.** `run-images.nu`
   stages `vendor/` into the cook context and folds a hash of the staged
   vendor tree into the toolchain gate hash, so a vendored code change
   forces a rebuild and the cook needs no GitHub auth for the forks.
8. **The local dev checkouts are no longer prerequisites.** They are
   deleted once the vendored build and the full suite are green; reading
   the engine is `vendor/<name>/…` in this repository.

## Consequences

- Positive: one repository for the change unit (consumer + engine), fork
  fixes in one commit, engine code reviewable next to its consumers, no
  100 GB of checkouts, a cook that cannot silently follow a moved branch,
  and a provenance battery with real teeth.
- Positive: `cargo` no longer needs network access for petri/pebble, so
  builds and CI are hermetic for the engine half.
- Named cost: the tree duplicates the fork's files (~20 MB, 1455 files).
  Accepted — it is the same source, and a `[patch]` that pointed at a
  local-only path would make the repository unbuildable for anyone else.
- Named cost: a vendored change is invisible to the fork until exported.
  The rule is the same as ADR-0027's dependency-finish rule: a session that
  patches a vendored crate exports it (or records why it cannot) in the
  same change.
- Named cost: the vendored workspaces carry their own lockfiles, which can
  lag fabro's shared dependencies. That is intentional isolation; a shared
  dependency that genuinely must move is moved in the fork and re-imported.
- Re-open condition: if upstream merges the fork patches and a released
  version covers them, the fork — and with it the vendor section — is
  dropped for the released dependency.
