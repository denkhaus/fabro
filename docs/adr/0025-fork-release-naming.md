# ADR-0025: Fork release naming `0.362.0-fork.N`

Date: 2026-10-02
Status: accepted (user decision 2026-10-02, "go für alles")
Companion to: ADR-0024 (platform freeze on 0.362)

## Context

The fork froze on upstream 0.362 (ADR-0024). Every deployed image still
carries the version string `0.362.0-nightly.0-<sha>` — an upstream version
that no longer describes anything we ship, and one that upstream itself has
moved past (0.374). The backend (`/system/info`, diagnostics) shows this
string; operators cannot tell which fork release is live beyond decoding
the commit sha.

## Decision

Fork releases are named `<frozen-base>-fork.<N>`:

- `<frozen-base>` = the upstream version the intake base froze on
  (today: `0.362.0`). It changes only when ADR-0024 intake moves the base.
- `N` = monotonic fork release counter, +1 per release we build.
- Image tags keep the sha suffix: `0.362.0-fork.1-a6cb9554c2`.
- First release under this scheme: `0.362.0-fork.1` (2026-10-02).

Rejected alternatives:
- Independent versioning from `0.1.0-fabro`: loses the upstream-parity
  anchor that the freeze era needs for cherry-pick intake.
- Calver `2026.10.x`: read margin is not worth the semver/cargo friction.

## Consequences

- The workspace `Cargo.toml` version is the fork version; the
  `0.362.0` prefix is deliberately frozen, not bumped per release.
- `cargo dev release` needs a fork mode that bumps only `N`, commits and
  tags on the line branch (`denkhaus`), and never pushes to `origin main`
  (tracked as a seed; until it lands, bumps are manual edits + 
  `cargo update --workspace`).
- Upstream tags (`v0.374.0-nightly.0`) never collide with fork tags
  (`v0.362.0-fork.N`).
