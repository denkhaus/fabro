-- Materialized observation triage read model (denkhaus/work#1385,
-- 2026-11-16): the derived triage state beside the observation records,
-- written by the write doors (record append, link, unlink, decision) so
-- the global feed answers a triage filter from SQL instead of re-deriving
-- per row with a whole-store walk at read time. The pair (run_id, seq) is
-- the observation record's append-only identity — the same key the link
-- and decision rows use; a side table, not a column on platform_records,
-- because that table is kind-generic and this state belongs to exactly
-- one kind.
--
-- Permanent schema (no removal deadline): the table is the read model's
-- home, not a compatibility shim — the write doors keep it current for
-- the feed's lifetime.
--
-- Backfill: existing records derive once from the best-effort inputs
-- available inside this database — link presence (work_observation_links)
-- and the latest decision row (observation_triage_decisions). A LINKED
-- record materializes `filed`: the live item state is unknowable inside a
-- SQL migration, and `filed` is the conservative default — the feed's
-- live overlay (the returned page re-resolves through the tracker) owns
-- the correction on display. Deliberately one-time: a tracker-side change
-- after this migration keeps the materialized state until a write door
-- touches the record (the stale corner the API description names).
--
-- The backfill joins the work tables by name: in a real deployment they
-- share this database file. Fresh fixture databases that install only the
-- records schema must install the work-table migrations too (the
-- fabro-store run-summary fixture does) — SQLite resolves the table names
-- at statement prepare time even against an empty database.

CREATE TABLE observation_triage_state (
    run_id        TEXT NOT NULL,
    seq           INTEGER NOT NULL,
    triage_state  TEXT NOT NULL,
    derived_at_ms INTEGER NOT NULL,
    PRIMARY KEY (run_id, seq),
    CHECK (seq >= 1),
    CHECK (triage_state IN ('untriaged', 'filed', 'in_progress', 'closed', 'reviewed'))
);

INSERT INTO observation_triage_state (run_id, seq, triage_state, derived_at_ms)
SELECT
    r.run_id,
    r.seq,
    CASE
        WHEN EXISTS (
            SELECT 1 FROM work_observation_links l
            WHERE l.run_id = r.run_id AND l.seq = r.seq
        ) THEN 'filed'
        WHEN EXISTS (
            SELECT 1 FROM observation_triage_decisions d
            WHERE d.run_id = r.run_id AND d.seq = r.seq
              AND d.decision = 'no_filing'
        ) THEN 'reviewed'
        ELSE 'untriaged'
    END,
    strftime('%s', 'now') * 1000
FROM platform_records r
WHERE r.kind = 'observation.recorded';
