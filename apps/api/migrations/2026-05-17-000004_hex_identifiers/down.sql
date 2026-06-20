DROP INDEX IF EXISTS idx_encounters_patient_type;
DROP INDEX IF EXISTS idx_encounters_provider_id;
DROP INDEX IF EXISTS idx_encounters_patient_id;
DROP INDEX IF EXISTS idx_providers_org_id;

DROP INDEX IF EXISTS idx_patients_hex_code_unique;
DROP INDEX IF EXISTS idx_sites_hex_code_unique;
DROP INDEX IF EXISTS idx_projects_hex_code_unique;
DROP INDEX IF EXISTS idx_organizations_hex_code_unique;

DROP TABLE IF EXISTS encounters CASCADE;
DROP TABLE IF EXISTS providers CASCADE;

ALTER TABLE patients DROP CONSTRAINT IF EXISTS patients_hex_code_format;
ALTER TABLE sites DROP CONSTRAINT IF EXISTS sites_hex_code_format;
ALTER TABLE projects DROP CONSTRAINT IF EXISTS projects_hex_code_format;
ALTER TABLE organizations DROP CONSTRAINT IF EXISTS organizations_hex_code_format;

ALTER TABLE patients DROP COLUMN IF EXISTS hex_code;
ALTER TABLE sites DROP COLUMN IF EXISTS hex_code;
ALTER TABLE projects DROP COLUMN IF EXISTS hex_code;
ALTER TABLE organizations DROP COLUMN IF EXISTS hex_code;
