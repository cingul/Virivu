DROP INDEX IF EXISTS idx_study_crf_fields_template_id;
DROP INDEX IF EXISTS idx_study_crf_templates_project_id;
DROP INDEX IF EXISTS idx_study_phase_events_project_id;
DROP INDEX IF EXISTS idx_projects_lifecycle_phase;

DROP TABLE IF EXISTS study_crf_fields CASCADE;
DROP TABLE IF EXISTS study_crf_templates CASCADE;
DROP TABLE IF EXISTS study_phase_events CASCADE;

ALTER TABLE projects DROP CONSTRAINT IF EXISTS projects_lifecycle_phase_valid;

ALTER TABLE projects DROP COLUMN IF EXISTS phase_changed_at;
ALTER TABLE projects DROP COLUMN IF EXISTS study_summary;
ALTER TABLE projects DROP COLUMN IF EXISTS clinicaltrials_gov_id;
ALTER TABLE projects DROP COLUMN IF EXISTS planned_enrollment;
ALTER TABLE projects DROP COLUMN IF EXISTS lifecycle_phase;
