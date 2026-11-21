-- The `filing_blocked` triage state (work#1585, 2026-11-21): the park
-- arm of the triage derivation. A failed filing write (rate limited,
-- 4xx) parks the observation under a `filing_blocked` decision for the
-- decision's bounded TTL; the board shows the park honestly instead of
-- pretending the record was never seen.
--
-- Permanent schema change (no removal deadline): the read model's
-- CHECK constraint enumerated the derivation's whole vocabulary, and
-- `filing_blocked` joins it. SQLite cannot alter a CHECK in place, so
-- this migration rebuilds the table — same columns, same key, the
-- widened vocabulary — and carries every materialized row across
-- unchanged (no re-derivation: the write doors own the derivation).

CREATE TABLE observation_triage_state_filing_blocked (
    run_id        TEXT NOT NULL,
    seq           INTEGER NOT NULL,
    triage_state  TEXT NOT NULL,
    derived_at_ms INTEGER NOT NULL,
    PRIMARY KEY (run_id, seq),
    CHECK (seq >= 1),
    CHECK (triage_state IN ('untriaged', 'filed', 'in_progress', 'closed', 'reviewed',
                            'filing_blocked'))
);

INSERT INTO observation_triage_state_filing_blocked (run_id, seq, triage_state, derived_at_ms)
    SELECT run_id, seq, triage_state, derived_at_ms FROM observation_triage_state;

DROP TABLE observation_triage_state;

ALTER TABLE observation_triage_state_filing_blocked RENAME TO observation_triage_state;
