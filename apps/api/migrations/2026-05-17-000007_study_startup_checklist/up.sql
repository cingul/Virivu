CREATE TABLE IF NOT EXISTS study_startup_checklist_items (
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

CREATE INDEX IF NOT EXISTS idx_study_startup_checklist_project_id
    ON study_startup_checklist_items(project_id);
