CREATE TABLE IF NOT EXISTS site_startup_checklist_items (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    site_id UUID NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    item_code TEXT NOT NULL,
    item_label TEXT NOT NULL,
    completed BOOLEAN NOT NULL DEFAULT FALSE,
    completed_by_user_id UUID,
    completed_at TIMESTAMPTZ,
    notes TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(site_id, item_code)
);

CREATE INDEX IF NOT EXISTS idx_site_startup_checklist_site_id
    ON site_startup_checklist_items(site_id);
