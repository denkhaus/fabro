-- Work-project run-target binding (ADR-0032 phase 3, 2026-10-21): a
-- project row may name the code repository its runs work on, so a run
-- resolves its project from its target (project-agnostic workflows: the
-- prompts name neither vendor, repository, label nor project id — the
-- run-scoped surface resolves `self` to the project bound to the run's
-- target repo). NULL keeps the project explicit-only.

ALTER TABLE work_projects ADD COLUMN target_repo TEXT;
