-- Runless Ask Fabro sessions (work#1677, ADR-0046 phase 2), part 2 of the
-- run_session_records rebuild (2026112501): run_session_events carries the
-- run anchor on every event, and the global supervisor's events hold none.
-- SQLite cannot ALTER a NOT NULL constraint away, so the events table is
-- rebuilt with the column nullable. WITHOUT ROWID and both indexes are
-- preserved exactly; no data changes — every existing row keeps its run id.

CREATE TABLE run_session_events_runless_20261125 (
    session_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    run_id TEXT,
    turn_id TEXT,
    event_name TEXT NOT NULL,
    recorded_at_ms INTEGER NOT NULL,
    properties_json TEXT NOT NULL,
    PRIMARY KEY (session_id, seq),
    CHECK (seq >= 1),
    CHECK (json_valid(properties_json))
) WITHOUT ROWID;

INSERT INTO run_session_events_runless_20261125 (
    session_id, seq, run_id, turn_id, event_name, recorded_at_ms, properties_json
)
SELECT session_id, seq, run_id, turn_id, event_name, recorded_at_ms, properties_json
FROM run_session_events;

DROP TABLE run_session_events;

ALTER TABLE run_session_events_runless_20261125
RENAME TO run_session_events;

CREATE INDEX run_session_events_by_run
ON run_session_events(run_id, session_id, seq);
