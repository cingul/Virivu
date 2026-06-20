DROP INDEX IF EXISTS idx_organizations_parent_org_id;
DROP INDEX IF EXISTS idx_organizations_workspace_slug_unique;

ALTER TABLE organizations DROP CONSTRAINT IF EXISTS organizations_kind_valid;
ALTER TABLE organizations DROP COLUMN IF EXISTS workspace_slug;
ALTER TABLE organizations DROP COLUMN IF EXISTS organization_kind;
ALTER TABLE organizations DROP COLUMN IF EXISTS parent_organization_id;
