CREATE TABLE IF NOT EXISTS organizations (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    workspace_slug TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS studies (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    short_code TEXT NOT NULL,
    title TEXT NOT NULL,
    phase TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (organization_id, short_code)
);

CREATE TABLE IF NOT EXISTS sites (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    study_id UUID NULL REFERENCES studies(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    principal_investigator TEXT NOT NULL,
    startup_complete BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS patients (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    study_id UUID NOT NULL REFERENCES studies(id) ON DELETE CASCADE,
    site_id UUID NULL REFERENCES sites(id) ON DELETE SET NULL,
    external_id TEXT NOT NULL,
    enrolled_at TIMESTAMPTZ NOT NULL,
    UNIQUE (study_id, external_id)
);

CREATE TABLE IF NOT EXISTS visits (
    id UUID PRIMARY KEY,
    study_id UUID NOT NULL REFERENCES studies(id) ON DELETE CASCADE,
    patient_id UUID NOT NULL REFERENCES patients(id) ON DELETE CASCADE,
    visit_name TEXT NOT NULL,
    scheduled_for TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS crf_templates (
    id UUID PRIMARY KEY,
    study_id UUID NOT NULL REFERENCES studies(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    published BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS crf_submissions (
    id UUID PRIMARY KEY,
    study_id UUID NOT NULL REFERENCES studies(id) ON DELETE CASCADE,
    patient_id UUID NOT NULL REFERENCES patients(id) ON DELETE CASCADE,
    visit_id UUID NOT NULL REFERENCES visits(id) ON DELETE CASCADE,
    template_id UUID NOT NULL REFERENCES crf_templates(id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    captured_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS data_queries (
    id UUID PRIMARY KEY,
    study_id UUID NOT NULL REFERENCES studies(id) ON DELETE CASCADE,
    submission_id UUID NOT NULL REFERENCES crf_submissions(id) ON DELETE CASCADE,
    summary TEXT NOT NULL,
    status TEXT NOT NULL,
    raised_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS dua_agreements (
    id UUID PRIMARY KEY,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    counterparty TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sites_study_id ON sites(study_id);
CREATE INDEX IF NOT EXISTS idx_patients_study_id ON patients(study_id);
CREATE INDEX IF NOT EXISTS idx_visits_study_patient ON visits(study_id, patient_id);
CREATE INDEX IF NOT EXISTS idx_submissions_study ON crf_submissions(study_id);
CREATE INDEX IF NOT EXISTS idx_queries_study_status ON data_queries(study_id, status);
