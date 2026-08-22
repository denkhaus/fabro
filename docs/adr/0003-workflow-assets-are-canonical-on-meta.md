# Workflow assets sync meta -> product, never the other way

`.fabro/workflows/`, `scripts/`, and `.mise.toml` are canonical on
`denkhaus-lab/meta` and copied to the product branch with a path-scoped
sync. The product justfile stays per-world on purpose: the product gate
checks product code, the platform gate checks workflow assets. `.seeds/`
is never synced (see ADR-0001), and the justfile is never synced (the gate
is each world's own contract).
