-- fabro-09ea: per-automation overlap policy for scheduled fires.
-- NULL keeps the unchanged default (fire regardless); 'skip' suppresses a
-- scheduled fire while a previous run of the same automation is still
-- non-terminal — including runs blocked at a human gate, which can wait
-- indefinitely by design (ADR-0011 amendment, 2026-09-02).
ALTER TABLE automations ADD COLUMN on_overlap TEXT
    CHECK (
        on_overlap IS NULL
        OR on_overlap IN ('fire', 'skip')
    );
