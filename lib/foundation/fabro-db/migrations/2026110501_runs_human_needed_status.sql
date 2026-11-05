-- #1072: the `human_needed` terminal status kind (the deadlock-exit
-- family parks a run as a decision request). SQLite cannot ALTER a CHECK
-- constraint, so the runs table is rebuilt with the extended status set.
-- No data changes: every existing row already carries a status in the
-- extended set; only the constraint widens. The shape mirrors the table
-- as 2026091803_drop_run_events.sql left it (the final column set).
CREATE TABLE runs_human_needed_20261105 (
    id TEXT PRIMARY KEY NOT NULL,
    created_at_ms INTEGER NOT NULL,
    started_at_ms INTEGER,
    last_event_at_ms INTEGER NOT NULL,
    completed_at_ms INTEGER,
    status TEXT NOT NULL,
    archived_at_ms INTEGER,
    parent_id TEXT,
    title TEXT NOT NULL,
    workflow_slug TEXT,
    workflow_name TEXT,
    repository_name TEXT,
    automation_id TEXT,
    diff_additions INTEGER NOT NULL DEFAULT 0,
    diff_deletions INTEGER NOT NULL DEFAULT 0,
    total_usd_micros INTEGER,
    summary_json TEXT NOT NULL,
    CHECK (status IN (
        'submitted',
        'pending',
        'runnable',
        'starting',
        'running',
        'blocked',
        'paused',
        'removing',
        'succeeded',
        'failed',
        'human_needed',
        'dead'
    )),
    CHECK (diff_additions >= 0),
    CHECK (diff_deletions >= 0),
    CHECK (total_usd_micros IS NULL OR total_usd_micros >= 0),
    CHECK (json_valid(summary_json))
);

INSERT INTO runs_human_needed_20261105 SELECT * FROM runs;

DROP TABLE runs;

ALTER TABLE runs_human_needed_20261105 RENAME TO runs;

CREATE INDEX runs_by_created_at ON runs(created_at_ms DESC, id DESC);
CREATE INDEX runs_by_updated_at ON runs(last_event_at_ms DESC, id DESC);
CREATE INDEX runs_by_status ON runs(archived_at_ms, status, last_event_at_ms DESC, id DESC);
CREATE INDEX runs_by_parent ON runs(parent_id, created_at_ms DESC, id DESC);
CREATE INDEX runs_by_automation ON runs(automation_id, created_at_ms DESC, id DESC);
