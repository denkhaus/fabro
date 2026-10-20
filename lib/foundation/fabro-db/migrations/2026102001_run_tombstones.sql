-- Removal tombstones (fabro-d616 arm b, 2026-10-20): deleting a run
-- writes its tombstone in the same transaction that drops the `runs`
-- row, and the projector's upsert refuses to re-insert a tombstoned run.
-- A late projection pass can no longer resurrect a deleted run as a
-- status=removing ghost (2026-10-19 incident: rm --force returned
-- success, the loop lane starved 10h on the resurrected row).

CREATE TABLE run_tombstones (
    run_id        TEXT PRIMARY KEY NOT NULL,
    deleted_at_ms INTEGER NOT NULL
);

CREATE INDEX run_tombstones_by_deleted_at ON run_tombstones(deleted_at_ms);
