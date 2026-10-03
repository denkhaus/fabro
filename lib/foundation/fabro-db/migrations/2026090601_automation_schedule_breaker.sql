-- fabro-3d97: automation schedule-trigger circuit breaker.
-- breaker_threshold is operator config (NULL = engine default of 3
-- consecutive same-signature failures). The remaining columns are scheduler
-- facts: the current consecutive-failure signature, the count, the last
-- terminal run processed (high-water mark), and the pause timestamp set when
-- the breaker disabled the trigger. Rewriting the trigger rows (the existing
-- create/replace path, used by the enable button) clears all facts.
ALTER TABLE automation_triggers ADD COLUMN breaker_threshold INTEGER
    CHECK (breaker_threshold IS NULL OR breaker_threshold >= 1);
ALTER TABLE automation_triggers ADD COLUMN breaker_signature TEXT;
ALTER TABLE automation_triggers ADD COLUMN breaker_consecutive_count INTEGER NOT NULL DEFAULT 0
    CHECK (breaker_consecutive_count >= 0);
ALTER TABLE automation_triggers ADD COLUMN breaker_last_run_id TEXT;
ALTER TABLE automation_triggers ADD COLUMN breaker_paused_at_ms INTEGER;
