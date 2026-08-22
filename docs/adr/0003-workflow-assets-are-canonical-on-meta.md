# The whole .fabro/workflows tree is canonical on meta and synced identical

`.fabro/workflows/` (every workflow, prompts, scripts) is canonical on
`meta/denkhaus-lab` and copied 1:1 to the product branch; the two trees
must never differ. Platform-only assets never sync to the product branch:
`.prime/` (reviewer-agent), `docs/fabro/` (engine docs snapshot),
`docs/reviews/` (review reports). Per-world on purpose: the justfile and
root `scripts/` gate different things (product code vs workflow lint);
`.mise.toml` provisions different toolchains (go vs fabro+uv). `.seeds/`
is never synced (ADR-0001). Decisions documentation is per-world (ADR-0005).
