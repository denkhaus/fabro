-- Runless Ask Fabro sessions (work#1677, ADR-0046 phase 2): the global
-- supervisor's conversations carry no run anchor, so run_session_records
-- and the session events that describe them hold a NULL run_id. SQLite
-- cannot ALTER a NOT NULL constraint away, so run_session_records is
-- rebuilt with the column nullable. No data changes: every existing row
-- keeps its run id. The run_session_events column is likewise made
-- nullable in its own rebuild (2026112502) — the two rebuilds together
-- are the runless session schema.

CREATE TABLE run_session_records_runless_20261125 (
    session_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT,
    record_json TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    CHECK (json_valid(record_json))
);

INSERT INTO run_session_records_runless_20261125 (
    session_id, run_id, record_json, updated_at_ms
)
SELECT session_id, run_id, record_json, updated_at_ms
FROM run_session_records;

DROP TABLE run_session_records;

ALTER TABLE run_session_records_runless_20261125
RENAME TO run_session_records;

CREATE INDEX run_session_records_by_run
ON run_session_records(run_id);
