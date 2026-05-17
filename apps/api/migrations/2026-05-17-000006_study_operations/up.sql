CREATE TABLE IF NOT EXISTS study_visit_templates (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    visit_code TEXT NOT NULL,
    visit_name TEXT NOT NULL,
    target_day INTEGER NOT NULL DEFAULT 0,
    window_before_days INTEGER NOT NULL DEFAULT 0,
    window_after_days INTEGER NOT NULL DEFAULT 0,
    required BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(project_id, visit_code)
);

CREATE TABLE IF NOT EXISTS patient_study_visits (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    patient_id UUID NOT NULL REFERENCES patients(id) ON DELETE CASCADE,
    visit_template_id UUID NOT NULL REFERENCES study_visit_templates(id) ON DELETE CASCADE,
    scheduled_for DATE,
    status TEXT NOT NULL DEFAULT 'scheduled',
    completed_at TIMESTAMPTZ,
    locked BOOLEAN NOT NULL DEFAULT FALSE,
    locked_at TIMESTAMPTZ,
    locked_by_user_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (status IN ('scheduled', 'completed', 'missed', 'cancelled'))
);

CREATE TABLE IF NOT EXISTS study_crf_submissions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    template_id UUID NOT NULL REFERENCES study_crf_templates(id) ON DELETE CASCADE,
    patient_id UUID NOT NULL REFERENCES patients(id) ON DELETE CASCADE,
    patient_visit_id UUID REFERENCES patient_study_visits(id) ON DELETE SET NULL,
    answers_json JSONB NOT NULL DEFAULT '{}'::JSONB,
    status TEXT NOT NULL DEFAULT 'draft',
    entered_by_user_id UUID,
    submitted_at TIMESTAMPTZ,
    locked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (status IN ('draft', 'submitted', 'locked'))
);

CREATE TABLE IF NOT EXISTS study_data_queries (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    submission_id UUID NOT NULL REFERENCES study_crf_submissions(id) ON DELETE CASCADE,
    field_key TEXT NOT NULL,
    query_text TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    response_text TEXT NOT NULL DEFAULT '',
    raised_by_user_id UUID,
    resolved_by_user_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (status IN ('open', 'responded', 'closed'))
);

CREATE TABLE IF NOT EXISTS study_close_checklist_items (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    item_code TEXT NOT NULL,
    item_label TEXT NOT NULL,
    completed BOOLEAN NOT NULL DEFAULT FALSE,
    completed_by_user_id UUID,
    completed_at TIMESTAMPTZ,
    notes TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(project_id, item_code)
);

CREATE INDEX IF NOT EXISTS idx_study_visit_templates_project_id
    ON study_visit_templates(project_id);
CREATE INDEX IF NOT EXISTS idx_patient_study_visits_project_id
    ON patient_study_visits(project_id);
CREATE INDEX IF NOT EXISTS idx_patient_study_visits_patient_id
    ON patient_study_visits(patient_id);
CREATE INDEX IF NOT EXISTS idx_study_crf_submissions_project_id
    ON study_crf_submissions(project_id);
CREATE INDEX IF NOT EXISTS idx_study_crf_submissions_patient_id
    ON study_crf_submissions(patient_id);
CREATE INDEX IF NOT EXISTS idx_study_data_queries_project_id
    ON study_data_queries(project_id);
CREATE INDEX IF NOT EXISTS idx_study_data_queries_submission_id
    ON study_data_queries(submission_id);
CREATE INDEX IF NOT EXISTS idx_study_close_checklist_project_id
    ON study_close_checklist_items(project_id);
