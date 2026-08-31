ALTER TABLE automations ADD COLUMN workflow_source_repository TEXT
    CHECK (
        workflow_source_repository IS NULL
        OR length(workflow_source_repository) BETWEEN 3 AND 140
    );

ALTER TABLE automations ADD COLUMN workflow_source_kind TEXT
    CHECK (
        workflow_source_kind IS NULL
        OR workflow_source_kind IN ('branch', 'tag', 'commit')
    );

ALTER TABLE automations ADD COLUMN workflow_source_ref TEXT
    CHECK (
        workflow_source_ref IS NULL
        OR length(workflow_source_ref) BETWEEN 1 AND 255
    );

CREATE TRIGGER automation_workflow_source_all_or_none_insert
BEFORE INSERT ON automations
WHEN
    (NEW.workflow_source_repository IS NULL)
    + (NEW.workflow_source_kind IS NULL)
    + (NEW.workflow_source_ref IS NULL) NOT IN (0, 3)
BEGIN
    SELECT RAISE(ABORT, 'automation workflow source must be entirely null or entirely present');
END;

CREATE TRIGGER automation_workflow_source_all_or_none_update
BEFORE UPDATE OF
    workflow_source_repository,
    workflow_source_kind,
    workflow_source_ref
ON automations
WHEN
    (NEW.workflow_source_repository IS NULL)
    + (NEW.workflow_source_kind IS NULL)
    + (NEW.workflow_source_ref IS NULL) NOT IN (0, 3)
BEGIN
    SELECT RAISE(ABORT, 'automation workflow source must be entirely null or entirely present');
END;
