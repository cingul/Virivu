CREATE TABLE data_use_agreements (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    organization_id UUID NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    hospital_name TEXT NOT NULL,
    hospital_contact_name TEXT NOT NULL DEFAULT '',
    hospital_contact_email TEXT NOT NULL DEFAULT '',
    counterparty_name TEXT NOT NULL DEFAULT 'Cingulum Foundation Inc.',
    agreement_version TEXT NOT NULL DEFAULT '1.0',
    status TEXT NOT NULL DEFAULT 'draft',
    effective_date DATE,
    expiration_date DATE,
    agreement_text TEXT NOT NULL,
    hospital_signing_token UUID NOT NULL UNIQUE,
    created_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    signed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (status IN ('draft', 'active', 'terminated', 'expired'))
);

CREATE TABLE data_use_agreement_signatures (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agreement_id UUID NOT NULL REFERENCES data_use_agreements(id) ON DELETE CASCADE,
    signer_role TEXT NOT NULL,
    signer_name TEXT NOT NULL,
    signer_email TEXT NOT NULL,
    signer_title TEXT NOT NULL DEFAULT '',
    signer_organization TEXT NOT NULL DEFAULT '',
    signature_method TEXT NOT NULL DEFAULT 'typed',
    signature_text TEXT NOT NULL,
    ip_address INET,
    signed_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    signed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (signer_role IN ('hospital', 'cingulum'))
);

CREATE UNIQUE INDEX idx_dua_signature_unique_role
    ON data_use_agreement_signatures(agreement_id, signer_role);
CREATE INDEX idx_dua_org_id ON data_use_agreements(organization_id);
CREATE INDEX idx_dua_status ON data_use_agreements(status);
CREATE INDEX idx_dua_signatures_agreement_id ON data_use_agreement_signatures(agreement_id);

CREATE TABLE outbound_emails (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    agreement_id UUID REFERENCES data_use_agreements(id) ON DELETE CASCADE,
    recipient_email TEXT NOT NULL,
    subject TEXT NOT NULL,
    body TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    requested_by_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CHECK (status IN ('queued', 'sent', 'failed'))
);

CREATE INDEX idx_outbound_emails_agreement_id ON outbound_emails(agreement_id);
CREATE INDEX idx_outbound_emails_status ON outbound_emails(status);
