# Sandbox GC — engine-side reclaim and one-off host cleanup

The engine garbage-collects run sandboxes and stale toolchain image tags
(fabro-44d8): a terminal develop run's `fabro-run-*` container is removed
once it is no longer revisable (workflow version superseded — the
stale-evidence definition in [`CONTEXT.md`](CONTEXT.md) — or older than the
age threshold), with the K most recent terminal sandboxes always kept, and
stale `ghcr.io/denkhaus/fabro-toolchain:<sha>` tags are untagged after a
refresh lands a new tag (current tag, `latest`, and the local
`fabro-toolchain:noble` build-cache image always survive). Implementation:
`lib/apps/fabro-server/src/sandbox_gc.rs` (fork-owned file, anchored in
`fork_seam_test.rs`) and `lib/components/fabro-sandbox/src/reclaim.rs`.

## One-off host-side relief (executed 2026-09-19)

The 90%-disk condition predated the engine fix. The commands below are the
one-off relief an operator can re-run on the docker host if the engine GC
is not yet deployed (or for a fresh accumulation). They mirror the engine
rules conservatively: keep the newest stopped run containers, all running
ones, and the current/newest toolchain tags.

```sh
# Stop nothing; remove all but the 5 newest STOPPED fabro-run containers.
docker ps -a --filter name='fabro-run-' --filter status=exited \
  --format '{{.ID}} {{.Names}}' \
  | sort -k2 | head -n -5 | cut -d' ' -f1 \
  | xargs -r docker rm

# Remove toolchain tags that are neither the one in use nor the newest.
# Inspect what runs first, then untag:
docker images 'ghcr.io/denkhaus/fabro-toolchain' --format '{{.Repository}}:{{.Tag}} {{.ID}}'
docker rmi ghcr.io/denkhaus/fabro-toolchain:<old-sha>   # keep :latest and the tag live containers use
```

Executed relief (2026-09-19, operator-approved): 202 stopped
`fabro-run-*` containers removed (5 newest kept), 6 old fabro server image
tags removed, one stale running sandbox
(`fabro-run-01M2DXVAJBY1XBWBSNKNG0M6DM`, failed 2026-09-13, stale-evidence)
removed after approval — 434G → 14G of 503G used. Residuals deliberately
left: orphaned run-workspace volumes (~985 MB) and ~2.6 GB dangling layers;
both are candidates for the retention supervisor (fabro-3377).
