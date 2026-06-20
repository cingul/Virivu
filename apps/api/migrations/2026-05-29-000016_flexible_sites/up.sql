-- Make sites more flexible: belong primarily to an Organization.
-- A site can optionally be associated with one specific Project/Study (for study-specific sites).
-- This decouples sites from studies so that:
--   - You can have sites with no studies yet
--   - You can have studies with no sites yet
--   - Sites can be general to an organization or assigned to studies

-- 1. Add organization_id (nullable first for safe migration)
ALTER TABLE sites
ADD COLUMN IF NOT EXISTS organization_id UUID;

-- 2. Backfill from existing project -> organization relationship
UPDATE sites s
SET organization_id = p.organization_id
FROM projects p
WHERE s.project_id = p.id
  AND s.organization_id IS NULL;

-- 3. Enforce NOT NULL + FK
ALTER TABLE sites
ALTER COLUMN organization_id SET NOT NULL;

ALTER TABLE sites
ADD CONSTRAINT sites_organization_id_fkey
FOREIGN KEY (organization_id) REFERENCES organizations(id) ON DELETE CASCADE;

-- 4. Make project_id nullable (the main decoupling change)
ALTER TABLE sites
ALTER COLUMN project_id DROP NOT NULL;

-- Helpful indexes for the new access patterns
CREATE INDEX IF NOT EXISTS idx_sites_organization_id ON sites(organization_id);
CREATE INDEX IF NOT EXISTS idx_sites_organization_project ON sites(organization_id, project_id);