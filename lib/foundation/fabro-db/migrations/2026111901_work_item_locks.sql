-- The operator item-lock door (denkhaus/work#1420, 2026-11-19): an
-- operator-held lock that blocks claiming while it stands. A side table
-- keyed (project_id, item_key), like work_observation_links — the lock
-- is assignment state that lives in fabro's database, NOT a tracker
-- label: it survives tracker round-trips and the forge never sees it.
-- The lane stays untouched; the lock rides beside it.
--
-- Consumers: the claim pool (claim_next) filters locked items out of
-- the candidate list (the stale-basis skip's layer, work#1377) and the
-- grant door (work_claims grant) refuses a locked item outright —
-- an already-held grant classifies first, so a live claim is never
-- changed by a lock placed after it.
--
-- No TTL by design: the lock stands until `fabro work unlock` lifts it
-- (the operator lifts it, no expiry job exists). Permanent schema (no
-- removal deadline) — operator state, not a compatibility shim. No
-- backfill: locks are created only by the lock door; every item starts
-- unlocked.

CREATE TABLE work_item_locks (
    project_id   TEXT NOT NULL REFERENCES work_projects(project_id),
    item_key     TEXT NOT NULL,
    locked_at_ms INTEGER NOT NULL,
    PRIMARY KEY (project_id, item_key)
);
