DROP INDEX IF EXISTS idx_study_close_checklist_project_id;
DROP INDEX IF EXISTS idx_study_data_queries_submission_id;
DROP INDEX IF EXISTS idx_study_data_queries_project_id;
DROP INDEX IF EXISTS idx_study_crf_submissions_patient_id;
DROP INDEX IF EXISTS idx_study_crf_submissions_project_id;
DROP INDEX IF EXISTS idx_patient_study_visits_patient_id;
DROP INDEX IF EXISTS idx_patient_study_visits_project_id;
DROP INDEX IF EXISTS idx_study_visit_templates_project_id;

DROP TABLE IF EXISTS study_close_checklist_items CASCADE;
DROP TABLE IF EXISTS study_data_queries CASCADE;
DROP TABLE IF EXISTS study_crf_submissions CASCADE;
DROP TABLE IF EXISTS patient_study_visits CASCADE;
DROP TABLE IF EXISTS study_visit_templates CASCADE;
