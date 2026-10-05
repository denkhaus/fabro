-- fabro-b869 step 5b (fork): per-trigger provider window facts, the
-- provider-window gate's read model. A JSON array of per-provider facts
-- (provider, window, last_probe_at, next_probe_at); NULL = the gate never
-- held this trigger's fires. Scheduler-maintained; create/replace clears
-- it (normalize_replace strips client-supplied facts, like the breaker).
ALTER TABLE automation_triggers ADD COLUMN provider_windows TEXT
    CHECK (provider_windows IS NULL OR json_valid(provider_windows));
