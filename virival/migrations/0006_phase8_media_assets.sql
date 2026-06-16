CREATE TABLE IF NOT EXISTS media_assets (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    study_id UUID NULL REFERENCES studies(id) ON DELETE SET NULL,
    patient_id UUID NULL REFERENCES patients(id) ON DELETE SET NULL,
    category TEXT NOT NULL,
    filename TEXT NOT NULL,
    object_key TEXT NOT NULL UNIQUE,
    content_type TEXT NOT NULL,
    byte_size BIGINT NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'pending_upload',
    upload_expires_at TIMESTAMPTZ NOT NULL,
    uploaded_at TIMESTAMPTZ NULL,
    created_by_user_id UUID NULL REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT media_assets_status_check CHECK (status IN ('pending_upload', 'uploaded'))
);

CREATE INDEX IF NOT EXISTS idx_media_assets_org ON media_assets(organization_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_media_assets_patient ON media_assets(patient_id);
CREATE INDEX IF NOT EXISTS idx_media_assets_study ON media_assets(study_id);
