ALTER TABLE projects
    ADD COLUMN IF NOT EXISTS lifecycle_phase TEXT NOT NULL DEFAULT 'pre_study';

ALTER TABLE projects
    ADD COLUMN IF NOT EXISTS planned_enrollment INTEGER NOT NULL DEFAULT 0;

ALTER TABLE projects
    ADD COLUMN IF NOT EXISTS clinicaltrials_gov_id TEXT;

ALTER TABLE projects
    ADD COLUMN IF NOT EXISTS study_summary TEXT NOT NULL DEFAULT '';

ALTER TABLE projects
    ADD COLUMN IF NOT EXISTS phase_changed_at TIMESTAMPTZ NOT NULL DEFAULT NOW();

CREATE INDEX IF NOT EXISTS idx_projects_lifecycle_phase
    ON projects (lifecycle_phase);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'projects_lifecycle_phase_valid'
    ) THEN
        ALTER TABLE projects
            ADD CONSTRAINT projects_lifecycle_phase_valid
            CHECK (
                lifecycle_phase IN (
                    'pre_study',
                    'initiated',
                    'active',
                    'monitoring',
                    'closed'
                )
            );
    END IF;
END
$$;

CREATE TABLE IF NOT EXISTS study_phase_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    previous_phase TEXT NOT NULL,
    new_phase TEXT NOT NULL,
    changed_by_user_id UUID,
    notes TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS study_crf_templates (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project_id UUID NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    version INTEGER NOT NULL DEFAULT 1,
    status TEXT NOT NULL DEFAULT 'draft',
    applicable_phase TEXT NOT NULL DEFAULT 'active',
    created_by_user_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (status IN ('draft', 'published', 'archived')),
    CHECK (
        applicable_phase IN ('pre_study', 'initiated', 'active', 'monitoring', 'closed')
    )
);

CREATE TABLE IF NOT EXISTS study_crf_fields (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    template_id UUID NOT NULL REFERENCES study_crf_templates(id) ON DELETE CASCADE,
    field_key TEXT NOT NULL,
    field_label TEXT NOT NULL,
    field_type TEXT NOT NULL,
    required BOOLEAN NOT NULL DEFAULT FALSE,
    options_json JSONB NOT NULL DEFAULT '[]'::JSONB,
    display_order INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (
        field_type IN (
            'text',
            'textarea',
            'number',
            'date',
            'datetime',
            'boolean',
            'single_select',
            'multi_select'
        )
    ),
    UNIQUE(template_id, field_key)
);

CREATE INDEX IF NOT EXISTS idx_study_phase_events_project_id
    ON study_phase_events (project_id);
CREATE INDEX IF NOT EXISTS idx_study_crf_templates_project_id
    ON study_crf_templates (project_id);
CREATE INDEX IF NOT EXISTS idx_study_crf_fields_template_id
    ON study_crf_fields (template_id);
