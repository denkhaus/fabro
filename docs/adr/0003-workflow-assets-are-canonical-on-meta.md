# Workflow assets sync meta -> product, never the other way

`.fabro/workflows/` and `.prime/agent/skills/` (the reviewer code)
are canonical on `meta/denkhaus-lab` and copied to the product branch
with a path-scoped sync. Everything else is per-world on
purpose: the product justfile and root scripts/ gate product code; the
platform justfile and root scripts/ lint workflow assets; `.mise.toml`
differs because the two worlds provision different toolchains (product:
go + just; platform: fabro + uv + just). `.seeds/` is never synced (see
ADR-0001). The justfile is never synced — the gate is each world's own
contract.
