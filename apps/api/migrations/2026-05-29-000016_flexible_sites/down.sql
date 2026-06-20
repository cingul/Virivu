-- Best-effort reversal. Note: If any sites were created with project_id = NULL after this
-- migration, the down migration cannot automatically restore them to a non-nullable state.

-- Drop the new indexes
DROP INDEX IF EXISTS idx_sites_organization_project;
DROP INDEX IF EXISTS idx_sites_organization_id;

-- Drop the FK and column we added
ALTER TABLE sites DROP CONSTRAINT IF EXISTS sites_organization_id_fkey;
ALTER TABLE sites DROP COLUMN IF EXISTS organization_id;

-- Restore project_id to NOT NULL (this will fail if any NULLs exist)
ALTER TABLE sites ALTER COLUMN project_id SET NOT NULL;