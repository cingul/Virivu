-- Extend media_upload_tickets for full entity coverage
ALTER TABLE media_upload_tickets ADD COLUMN IF NOT EXISTS entity_type VARCHAR(64) NOT NULL DEFAULT 'patient';
ALTER TABLE media_upload_tickets ADD COLUMN IF NOT EXISTS entity_id UUID;
ALTER TABLE media_upload_tickets ADD COLUMN IF NOT EXISTS file_name VARCHAR(256);
ALTER TABLE media_upload_tickets ADD COLUMN IF NOT EXISTS description VARCHAR(512);
ALTER TABLE media_upload_tickets ADD COLUMN IF NOT EXISTS upload_status VARCHAR(32) NOT NULL DEFAULT 'pending';

CREATE INDEX IF NOT EXISTS idx_media_tickets_entity ON media_upload_tickets (entity_type, entity_id);
