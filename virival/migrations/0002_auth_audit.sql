CREATE TABLE IF NOT EXISTS users (
    id UUID PRIMARY KEY,
    subject TEXT NOT NULL UNIQUE,
    email TEXT NULL,
    display_name TEXT NULL,
    platform_role TEXT NOT NULL DEFAULT 'investigator',
    created_at TIMESTAMPTZ NOT NULL
);

CREATE TABLE IF NOT EXISTS organization_memberships (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    UNIQUE (user_id, organization_id)
);

CREATE TABLE IF NOT EXISTS audit_logs (
    id UUID PRIMARY KEY,
    actor_user_id UUID NULL REFERENCES users(id) ON DELETE SET NULL,
    actor_subject TEXT NOT NULL,
    actor_role TEXT NULL,
    actor_email TEXT NULL,
    auth_source TEXT NULL,
    method TEXT NOT NULL,
    path TEXT NOT NULL,
    status_code INTEGER NOT NULL,
    happened_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_memberships_org_id ON organization_memberships(organization_id);
CREATE INDEX IF NOT EXISTS idx_memberships_user_id ON organization_memberships(user_id);
CREATE INDEX IF NOT EXISTS idx_audit_logs_happened_at ON audit_logs(happened_at DESC);
CREATE INDEX IF NOT EXISTS idx_audit_logs_actor_subject ON audit_logs(actor_subject);
