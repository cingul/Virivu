-- Revert indices
DROP INDEX IF EXISTS idx_sites_status_activity;
DROP INDEX IF EXISTS idx_projects_status_activity;

-- Revert columns
ALTER TABLE sites 
DROP COLUMN IF EXISTS last_activity_at,
DROP COLUMN IF EXISTS status;

ALTER TABLE projects 
DROP COLUMN IF EXISTS last_activity_at,
DROP COLUMN IF EXISTS status;
