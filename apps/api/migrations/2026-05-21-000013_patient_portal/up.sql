-- Patient portal: magic-link sessions
CREATE TABLE IF NOT EXISTS patient_sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    patient_id UUID NOT NULL REFERENCES patients(id) ON DELETE CASCADE,
    token TEXT NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL DEFAULT NOW() + INTERVAL '7 days',
    used_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_patient_sessions_token ON patient_sessions(token);
CREATE INDEX IF NOT EXISTS idx_patient_sessions_patient ON patient_sessions(patient_id);

-- PRO submissions (patient-reported outcomes)
CREATE TABLE IF NOT EXISTS pro_submissions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    patient_id UUID NOT NULL REFERENCES patients(id) ON DELETE CASCADE,
    form_type TEXT NOT NULL CHECK (form_type IN ('phq9', 'gad7', 'sf36', 'eq5d', 'promis', 'custom')),
    answers JSONB NOT NULL DEFAULT '{}',
    total_score INTEGER,
    submitted_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_pro_submissions_patient ON pro_submissions(patient_id);
