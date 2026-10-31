-- Work-item observation links (ADR-0038, 2026-10-31): the closure-radius
-- chain's link edge. A work item links to an OBSERVATION record — the
-- pair (run_id, seq) is exactly the record's append-only identity — and
-- the link lives ONLY in fabro's database: the platform owns both ends
-- (the tracker keeps items, fabro keeps the chain; no GitHub mirror, a
-- mirror is a drift surface).
--
-- Writers: the user-only work link surface (backfill, bench) today; the
-- run-end hook (ADR-0038 stage 4) writes links internally server-side
-- when it files items from a run's observation records, deduping at the
-- link. The close verb derives a claimed item's closure radius from
-- these rows: its runs' observations' linked OPEN items.

CREATE TABLE work_observation_links (
    project_id  TEXT NOT NULL REFERENCES work_projects(project_id),
    item_key    TEXT NOT NULL,
    run_id      TEXT NOT NULL,
    seq         INTEGER NOT NULL,
    linked_at_ms INTEGER NOT NULL,
    -- One link per (item, observation): filing dedupes at the link — an
    -- observation already linked to an item is linked, never recreated.
    PRIMARY KEY (project_id, item_key, run_id, seq)
);

-- The reverse join: an observation's linked items (the hook's dedupe
-- lookup and the radius traversal both walk observation -> items).
CREATE INDEX work_observation_links_by_observation
    ON work_observation_links(run_id, seq);
