ALTER TABLE organizations
    ADD COLUMN IF NOT EXISTS parent_organization_id UUID REFERENCES organizations(id) ON DELETE SET NULL;

ALTER TABLE organizations
    ADD COLUMN IF NOT EXISTS organization_kind TEXT NOT NULL DEFAULT 'tenant';

ALTER TABLE organizations
    ADD COLUMN IF NOT EXISTS workspace_slug TEXT;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'organizations_kind_valid'
    ) THEN
        ALTER TABLE organizations
            ADD CONSTRAINT organizations_kind_valid
            CHECK (
                organization_kind IN (
                    'platform_root',
                    'research_network',
                    'hospital',
                    'tenant',
                    'sponsor'
                )
            );
    END IF;
END
$$;

CREATE UNIQUE INDEX IF NOT EXISTS idx_organizations_workspace_slug_unique
    ON organizations (workspace_slug)
    WHERE workspace_slug IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_organizations_parent_org_id
    ON organizations (parent_organization_id);

DO $$
DECLARE
    root_id UUID;
BEGIN
    SELECT id INTO root_id
    FROM organizations
    WHERE workspace_slug = 'cingulum-foundation'
       OR organization_kind = 'platform_root'
    ORDER BY created_at ASC
    LIMIT 1;

    IF root_id IS NULL THEN
        INSERT INTO organizations (name, organization_kind, workspace_slug)
        VALUES ('Cingulum Foundation Inc.', 'platform_root', 'cingulum-foundation')
        RETURNING id INTO root_id;
    ELSE
        UPDATE organizations
        SET organization_kind = 'platform_root',
            workspace_slug = COALESCE(workspace_slug, 'cingulum-foundation')
        WHERE id = root_id;
    END IF;

    UPDATE organizations
    SET parent_organization_id = root_id
    WHERE id <> root_id
      AND parent_organization_id IS NULL
      AND organization_kind <> 'platform_root';
END
$$;
