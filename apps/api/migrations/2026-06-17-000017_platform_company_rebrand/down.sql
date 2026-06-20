UPDATE organizations
SET name = 'Cingulum Foundation Inc.',
    workspace_slug = 'cingulum-foundation'
WHERE organization_kind = 'platform_root';

UPDATE data_use_agreements
SET counterparty_name = 'Cingulum Foundation Inc.'
WHERE counterparty_name = 'Virivu Research Cloud Services, Inc.';

ALTER TABLE data_use_agreements
    ALTER COLUMN counterparty_name SET DEFAULT 'Cingulum Foundation Inc.';

UPDATE data_use_agreement_signatures
SET signer_organization = 'Cingulum Foundation Inc.'
WHERE signer_role = 'cingulum'
  AND signer_organization = 'Virivu Research Cloud Services, Inc.';
