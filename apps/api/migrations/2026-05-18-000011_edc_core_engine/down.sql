

ALTER TABLE study_crf_submissions
DROP COLUMN IF EXISTS sdv_status;

ALTER TABLE study_crf_fields 
DROP COLUMN IF EXISTS edit_checks_json,
DROP COLUMN IF EXISTS branching_logic_json;

DROP TABLE IF EXISTS audit_logs;
