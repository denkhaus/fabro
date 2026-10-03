# The whole .fabro/workflows tree is canonical on meta and synced identical

`.fabro/workflows/` (every workflow, prompts, scripts) is canonical on
`meta/denkhaus-lab` and copied 1:1 to the product branch; the two trees
must never differ. Platform-only assets never sync to the product branch:
`.prime/` (reviewer-agent), `docs/fabro/` (engine docs snapshot),
`docs/reviews/` (review reports). Per-world on purpose: the justfile and
root `scripts/` gate different things (product code vs workflow lint);
`.mise.toml` provisions different toolchains (go vs fabro+uv). `.seeds/`
is never synced (ADR-0001). Decisions documentation is per-world (ADR-0005).

Enforcement is mechanical: `develop/scripts/sync-check.nu` lives inside the
synced tree (it cannot itself drift) and runs as the first step of BOTH
worlds' quality gates. It pairs `meta/<name>` with `<name>`, skips loudly
on non-world branches (run branches, detached heads, single-world repos),
and fails the gate with a diff stat plus the re-sync command when the
trees diverge. Drift therefore cannot pass a gate in either world.
