-- The work-item read-view materialization (ADR-0050, work#1557, 2026-11-21):
-- the server holds a materialized READ VIEW of work-item content beside
-- the claim ledger. Reads resolve cache-first; the forge loads on demand.
-- Write-invalidation only, no TTL (operator choice): a row is fresh until
-- one of fabro's own writes moves it (write-through) or an explicit
-- refresh runs. Foreign edits stay invisible until the refresh door —
-- accepted deliberately for maximum quota savings.
--
-- `work_item_views` is the content row: first cut is item content only
-- (title, body, labels, state) plus the identity and rank fields the
-- list assembly needs (priority, url, timestamps). `content_hash` names
-- whether a re-fetch moved the content (the refresh door's changed
-- count); `fetched_at_ms` is the honest age of the row. Comments and
-- links are part 2 and live nowhere here.
--
-- `work_item_universe` is the per-project key set the list assembly
-- walks: seeded by one full state=all forge walk on first listing,
-- maintained by writes and the refresh door. Its emptiness is the
-- "not yet materialized" marker — an empty universe makes the next list
-- seed it.
--
-- Permanent schema (no removal deadline): this is the read-view store
-- ADR-0050 decides on, not a compatibility shim. No backfill: rows
-- appear only through the materializing adapter; every project starts
-- unmaterialized and seeds on first read.

CREATE TABLE work_item_views (
    project_id    TEXT NOT NULL,
    item_key      TEXT NOT NULL,
    title         TEXT NOT NULL,
    body          TEXT NOT NULL,
    state         TEXT NOT NULL,
    labels_json   TEXT NOT NULL,
    priority      INTEGER NOT NULL,
    url           TEXT NOT NULL,
    updated_at_ms INTEGER,
    created_at_ms INTEGER,
    closed_at_ms  INTEGER,
    content_hash  TEXT NOT NULL,
    fetched_at_ms INTEGER NOT NULL,
    PRIMARY KEY (project_id, item_key)
);

CREATE TABLE work_item_universe (
    project_id    TEXT NOT NULL,
    item_key      TEXT NOT NULL,
    first_seen_ms INTEGER NOT NULL,
    seen_at_ms    INTEGER NOT NULL,
    PRIMARY KEY (project_id, item_key)
);
