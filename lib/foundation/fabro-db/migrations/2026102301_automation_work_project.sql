-- Automation work-project reference (fabro-f1a9, ADR-0032 Phase 6,
-- 2026-10-23): an automation names the work project its runs claim
-- through — the association is a reference, not a string match between
-- targetRepository and the project's targetRepo (join-design review
-- 2026-10-23). The scheduler consults it as a cheap binding health gate
-- before firing: a dead or missing binding refuses the fire instead of
-- burning a run against it (the overnight tracker-binding outage).
-- NULL on legacy rows until an operator sets it (the five line
-- automations are set manually in the deploy window); creating a new
-- automation requires it (API validation).

ALTER TABLE automations ADD COLUMN work_project TEXT;
