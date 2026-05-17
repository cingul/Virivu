ALTER TABLE organizations
    ADD COLUMN IF NOT EXISTS hex_code TEXT;

ALTER TABLE projects
    ADD COLUMN IF NOT EXISTS hex_code TEXT;

ALTER TABLE sites
    ADD COLUMN IF NOT EXISTS hex_code TEXT;

ALTER TABLE patients
    ADD COLUMN IF NOT EXISTS hex_code TEXT;

CREATE TABLE IF NOT EXISTS providers (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    referral_source TEXT NOT NULL DEFAULT '',
    hex_code TEXT NOT NULL UNIQUE CHECK (hex_code ~ '^[0-9A-F]{6}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS encounters (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    patient_id UUID NOT NULL REFERENCES patients(id) ON DELETE CASCADE,
    provider_id UUID REFERENCES providers(id) ON DELETE SET NULL,
    encounter_type TEXT NOT NULL CHECK (
        encounter_type IN (
            'outpatient',
            'inpatient',
            'labs',
            'imaging',
            'procedures',
            'misc'
        )
    ),
    notes TEXT NOT NULL DEFAULT '',
    hex_code TEXT NOT NULL UNIQUE CHECK (hex_code ~ '^[0-9A-F]{16}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_organizations_hex_code_unique
    ON organizations (hex_code)
    WHERE hex_code IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_projects_hex_code_unique
    ON projects (hex_code)
    WHERE hex_code IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_sites_hex_code_unique
    ON sites (hex_code)
    WHERE hex_code IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_patients_hex_code_unique
    ON patients (hex_code)
    WHERE hex_code IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_providers_org_id ON providers (organization_id);
CREATE INDEX IF NOT EXISTS idx_encounters_patient_id ON encounters (patient_id);
CREATE INDEX IF NOT EXISTS idx_encounters_provider_id ON encounters (provider_id);
CREATE INDEX IF NOT EXISTS idx_encounters_patient_type ON encounters (patient_id, encounter_type);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'organizations_hex_code_format'
    ) THEN
        ALTER TABLE organizations
            ADD CONSTRAINT organizations_hex_code_format
            CHECK (
                hex_code IS NULL
                OR (hex_code ~ '^[0-9A-F]{3}$' AND hex_code ~ '[A-F]')
            );
    END IF;
END
$$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'projects_hex_code_format'
    ) THEN
        ALTER TABLE projects
            ADD CONSTRAINT projects_hex_code_format
            CHECK (
                hex_code IS NULL
                OR (hex_code ~ '^[0-9A-F]{6}$' AND hex_code ~ '[A-F]')
            );
    END IF;
END
$$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'sites_hex_code_format'
    ) THEN
        ALTER TABLE sites
            ADD CONSTRAINT sites_hex_code_format
            CHECK (
                hex_code IS NULL
                OR (hex_code ~ '^[0-9A-F]{9}$' AND hex_code ~ '[A-F]')
            );
    END IF;
END
$$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'patients_hex_code_format'
    ) THEN
        ALTER TABLE patients
            ADD CONSTRAINT patients_hex_code_format
            CHECK (
                hex_code IS NULL
                OR (hex_code ~ '^[0-9A-F]{13}$' AND hex_code ~ '[A-F]')
            );
    END IF;
END
$$;
