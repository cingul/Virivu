CREATE TABLE IF NOT EXISTS dua_signatures (
    id UUID PRIMARY KEY,
    dua_agreement_id UUID NOT NULL REFERENCES dua_agreements(id) ON DELETE CASCADE,
    signer_name TEXT NOT NULL,
    signer_email TEXT NOT NULL,
    signer_role TEXT NOT NULL,
    signature_text TEXT NOT NULL,
    signed_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_dua_signatures_dua_id ON dua_signatures(dua_agreement_id);

CREATE TABLE IF NOT EXISTS reminder_jobs (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    study_id UUID NULL REFERENCES studies(id) ON DELETE SET NULL,
    patient_id UUID NULL REFERENCES patients(id) ON DELETE SET NULL,
    visit_id UUID NULL REFERENCES visits(id) ON DELETE SET NULL,
    channel TEXT NOT NULL,
    recipient TEXT NOT NULL,
    message TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    scheduled_for TIMESTAMPTZ NOT NULL,
    processed_at TIMESTAMPTZ NULL,
    last_error TEXT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT reminder_jobs_status_check CHECK (status IN ('pending', 'sent', 'failed'))
);

CREATE INDEX IF NOT EXISTS idx_reminder_jobs_status_scheduled
    ON reminder_jobs(status, scheduled_for);
