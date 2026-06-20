ALTER TABLE audit_logs
    ADD COLUMN IF NOT EXISTS action TEXT NULL,
    ADD COLUMN IF NOT EXISTS resource_type TEXT NULL,
    ADD COLUMN IF NOT EXISTS resource_id UUID NULL,
    ADD COLUMN IF NOT EXISTS metadata_json JSONB NULL;

CREATE INDEX IF NOT EXISTS idx_audit_logs_action ON audit_logs(action);
CREATE INDEX IF NOT EXISTS idx_audit_logs_resource_type ON audit_logs(resource_type);
CREATE INDEX IF NOT EXISTS idx_audit_logs_resource_id ON audit_logs(resource_id);
