-- Work-item tracker and claim ledger (ADR-0032, 2026-10-20): work-item
-- content lives in a tracker provider (first adapter: one central private
-- GitHub issue repo); assignment/queue state lives here. A non-terminal
-- claim is unique per (project, item) and per run, so two runs can never
-- both own the same work item regardless of which clone they read.
--
-- `work_outbox` is the durable projection queue: the claim transaction
-- commits first, the tracker label/issue write happens later and retries.

CREATE TABLE work_projects (
    project_id            TEXT PRIMARY KEY NOT NULL,
    display_name          TEXT NOT NULL,
    tracker_provider      TEXT NOT NULL,
    tracker_repo          TEXT NOT NULL,
    project_label         TEXT NOT NULL,
    lane_labels_json      TEXT NOT NULL DEFAULT '{}',
    claim_label_prefix    TEXT NOT NULL DEFAULT 'fabro:run:',
    claim_comment_enabled INTEGER NOT NULL DEFAULT 0,
    backpressure_open_pr_cap INTEGER NOT NULL DEFAULT 0,
    created_at_ms         INTEGER NOT NULL,
    updated_at_ms         INTEGER NOT NULL
);

CREATE TABLE work_claims (
    claim_id        TEXT PRIMARY KEY NOT NULL,
    project_id      TEXT NOT NULL REFERENCES work_projects(project_id),
    item_key        TEXT NOT NULL,
    run_id          TEXT NOT NULL,
    lane            TEXT NOT NULL,
    state           TEXT NOT NULL,
    granted_at_ms   INTEGER NOT NULL,
    updated_at_ms   INTEGER NOT NULL,
    released_at_ms  INTEGER,
    release_reason  TEXT,
    pr_url          TEXT
);

-- One non-terminal claim per work item. `awaiting_merge` still holds the
-- item: a pending pull request is not available for a new run.
CREATE UNIQUE INDEX work_claims_one_live_per_item
    ON work_claims(project_id, item_key)
    WHERE state IN ('granted', 'active', 'awaiting_merge');

-- One non-terminal claim per run: a run works on exactly one item.
CREATE UNIQUE INDEX work_claims_one_live_per_run
    ON work_claims(run_id)
    WHERE state IN ('granted', 'active', 'awaiting_merge');

CREATE INDEX work_claims_by_run_state ON work_claims(run_id, state);
CREATE INDEX work_claims_by_project_state ON work_claims(project_id, state);
CREATE INDEX work_claims_by_granted_at ON work_claims(granted_at_ms);

CREATE TABLE work_claim_events (
    claim_id   TEXT NOT NULL REFERENCES work_claims(claim_id),
    seq        INTEGER NOT NULL,
    at_ms      INTEGER NOT NULL,
    from_state TEXT,
    to_state   TEXT NOT NULL,
    reason     TEXT,
    run_id     TEXT NOT NULL,
    PRIMARY KEY (claim_id, seq)
);

CREATE TABLE work_outbox (
    id                TEXT PRIMARY KEY NOT NULL,
    idempotency_key   TEXT NOT NULL UNIQUE,
    kind              TEXT NOT NULL,
    payload_json      TEXT NOT NULL,
    attempts          INTEGER NOT NULL DEFAULT 0,
    next_attempt_at_ms INTEGER NOT NULL,
    created_at_ms     INTEGER NOT NULL,
    completed_at_ms   INTEGER,
    last_error        TEXT
);

CREATE INDEX work_outbox_due
    ON work_outbox(next_attempt_at_ms)
    WHERE completed_at_ms IS NULL;
