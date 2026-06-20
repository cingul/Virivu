DROP INDEX IF EXISTS idx_media_tickets_entity;
ALTER TABLE media_upload_tickets DROP COLUMN IF EXISTS upload_status;
ALTER TABLE media_upload_tickets DROP COLUMN IF EXISTS description;
ALTER TABLE media_upload_tickets DROP COLUMN IF EXISTS file_name;
ALTER TABLE media_upload_tickets DROP COLUMN IF EXISTS entity_id;
ALTER TABLE media_upload_tickets DROP COLUMN IF EXISTS entity_type;
