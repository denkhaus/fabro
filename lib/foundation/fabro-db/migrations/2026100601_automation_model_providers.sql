-- fabro-0611 (fork, b869 step 3): the admission-derived model providers a
-- never-run automation's workflow requires, learned at the fire's admission
-- (create path) so the provider-window gate can probe, hold and write facts
-- for automations with no terminal run yet. A JSON array of
-- {provider, model} entries; NULL = the automation never fired (or the
-- workflow consults no model). Scheduler/create-path-maintained; replace
-- clears it (the workflow may have changed); never part of the revision.
ALTER TABLE automations ADD COLUMN model_providers TEXT
    CHECK (model_providers IS NULL OR json_valid(model_providers));
