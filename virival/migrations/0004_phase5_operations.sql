CREATE TABLE IF NOT EXISTS crf_template_versions (
    id UUID PRIMARY KEY,
    template_id UUID NOT NULL REFERENCES crf_templates(id) ON DELETE CASCADE,
    version_number INTEGER NOT NULL,
    schema_json JSONB NOT NULL,
    is_published BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (template_id, version_number)
);

CREATE TABLE IF NOT EXISTS visit_schedule_templates (
    id UUID PRIMARY KEY,
    study_id UUID NOT NULL REFERENCES studies(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    day_offset INTEGER NOT NULL,
    window_before_days INTEGER NOT NULL DEFAULT 0,
    window_after_days INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS data_query_comments (
    id UUID PRIMARY KEY,
    query_id UUID NOT NULL REFERENCES data_queries(id) ON DELETE CASCADE,
    author_user_id UUID NULL REFERENCES users(id) ON DELETE SET NULL,
    comment_text TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS study_closeout_checklist_items (
    id UUID PRIMARY KEY,
    study_id UUID NOT NULL REFERENCES studies(id) ON DELETE CASCADE,
    item_key TEXT NOT NULL,
    item_label TEXT NOT NULL,
    is_required BOOLEAN NOT NULL DEFAULT TRUE,
    is_complete BOOLEAN NOT NULL DEFAULT FALSE,
    completed_at TIMESTAMPTZ NULL,
    completed_by_user_id UUID NULL REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (study_id, item_key)
);

CREATE INDEX IF NOT EXISTS idx_crf_template_versions_template_id ON crf_template_versions(template_id);
CREATE INDEX IF NOT EXISTS idx_crf_template_versions_published ON crf_template_versions(template_id, is_published);
CREATE INDEX IF NOT EXISTS idx_visit_schedule_templates_study_id ON visit_schedule_templates(study_id);
CREATE INDEX IF NOT EXISTS idx_data_query_comments_query_id ON data_query_comments(query_id);
CREATE INDEX IF NOT EXISTS idx_closeout_items_study_id ON study_closeout_checklist_items(study_id);
