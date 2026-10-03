# Revision — run 01M2TJ9JZGE42JNBJ22A258MBZ

- status reviewed: succeeded
- review: .fabro/reviews/develop/01M2TJ9JZGE42JNBJ22A258MBZ.md
- seeds filed: fabro-d016 (reopen java/openapi-generator bake — hit fabro-3b1b's documented reopen trigger), fabro-7d98 (mtime touch at verify.nu start, pre-gate arm)
- basis: run 01M2TJ9JZGE42JNBJ22A258MBZ, workflow version cbb412e5e0b00bea7cb3ab8e1e5b2e3d853858bf1d497ac1e86fa15438141bbc, commit 94086b06d81a673c4e2c52ba80668bb0e997be27
- revised_at_commit: 94086b06d81a673c4e2c52ba80668bb0e997be27 (ADR-0015: engine drift signal for later judgement)

## Findings

### 1. Reopen fabro-3b1b (java in run image) — filed fabro-d016
Run 01M2TJ9JZG (fabro-e566, PR #240) hit `java: not found`, blocking the acceptance criterion's
`bun run generate` TS regen; the implementer hand-mirrored the generated diff as a disclosed
deviation. That is exactly fabro-3b1b's documented reopen condition. Filed fabro-d016 as
`needs-user,revision` (ADR-0019: capability-affecting, and the 2026-09-17 user decision said
"no java"); fabro-3b1b itself left closed — reopening a user-decided seed is the human gate's
call, the new seed carries the evidence. Not covered by fabro-7aac (deferred follow-up re-filing)
or any open seed.

### 2. Mtime touch at verify.nu start — filed fabro-7d98
The stale-mtime trap bit pre-gate this run (stale binaries, ~4 wasted diagnostic calls; lesson
mx-6376dd), but fabro-56db's touch lives only in `qualitygate.nu`. Filed fabro-7d98 to add the
same one-liner at the start of `.fabro/workflows/develop/scripts/verify.nu`. Different file and
mechanism point than fabro-56db (open, cross-referenced, not superseded); closed fabro-22e4 is
the transport root cause, not this mitigation.

### 3. fabro-c643 anchor typo — no action needed (already fixed)
The finding asked to fix a `Dockerfile.toolchai` typo in fabro-c643's body. Inspection of the
current tracker record shows all 10 `toolchai*` occurrences already read `toolchain` — the body
is correct as of commit 94086b06. Nothing rewritten; if the planner preflight still flags
`anchors_ok: false`, that is the checker side (open fabro-7611), not the seed body.
