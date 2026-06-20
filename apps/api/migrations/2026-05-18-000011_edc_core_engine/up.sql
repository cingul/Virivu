CREATE TABLE IF NOT EXISTS audit_logs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    entity_table TEXT NOT NULL,
    entity_id UUID NOT NULL,
    action TEXT NOT NULL,
    changed_by_user_id UUID,
    old_data JSONB,
    new_data JSONB,
    reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_audit_logs_entity
    ON audit_logs(entity_table, entity_id);

ALTER TABLE study_crf_fields 
ADD COLUMN IF NOT EXISTS branching_logic_json JSONB NOT NULL DEFAULT 'null'::JSONB,
ADD COLUMN IF NOT EXISTS edit_checks_json JSONB NOT NULL DEFAULT 'null'::JSONB;

ALTER TABLE study_crf_submissions
ADD COLUMN IF NOT EXISTS sdv_status TEXT NOT NULL DEFAULT 'unverified';

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'study_crf_submissions_sdv_status_check'
    ) THEN
        ALTER TABLE study_crf_submissions
            ADD CONSTRAINT study_crf_submissions_sdv_status_check
            CHECK (
                sdv_status IN (
                    'unverified',
                    'verified',
                    'needs_attention'
                )
            );
    END IF;
END
$$;


