-- Observation triage decisions (ADR-0044, 2026-11-15): the triager's
-- keyed decision row. A triage decision is ABOUT one observation record
-- — the pair (run_id, seq) is the record's append-only identity, unique
-- across the store — so the row keys on the observation alone; no
-- project scope (that is the link row's shape, and a decision files no
-- item).
--
-- Writers: the manage door (POST /api/v1/observations/manage) — user
-- principals always, a workflow run only when its workflow is the
-- triager (the mark door's allowlist posture). Upsert semantics: the
-- latest decision on a record wins; a work-item link beats the decision
-- wherever both exist (a later filing overrides a no_filing).

CREATE TABLE observation_triage_decisions (
    run_id       TEXT NOT NULL,
    seq          INTEGER NOT NULL,
    decision     TEXT NOT NULL,
    reason       TEXT NOT NULL,
    decided_at_ms INTEGER NOT NULL,
    by_run       TEXT,
    -- One decision per observation: the upsert keeps the latest.
    PRIMARY KEY (run_id, seq)
);

-- The triage derivation reads a run's decisions beside its link rows.
CREATE INDEX observation_triage_decisions_by_run
    ON observation_triage_decisions(run_id);
