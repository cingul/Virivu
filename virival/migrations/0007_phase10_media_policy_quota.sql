CREATE TABLE IF NOT EXISTS organization_media_policies (
    organization_id UUID PRIMARY KEY REFERENCES organizations(id) ON DELETE CASCADE,
    max_total_bytes BIGINT NULL,
    max_asset_bytes BIGINT NULL,
    updated_at TIMESTAMPTZ NOT NULL
);

ALTER TABLE media_assets
    ADD COLUMN IF NOT EXISTS expected_byte_size BIGINT NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_media_assets_org_status
    ON media_assets(organization_id, status);
