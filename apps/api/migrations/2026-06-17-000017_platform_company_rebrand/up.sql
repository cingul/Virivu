UPDATE organizations
SET name = 'Virivu Research Cloud Services, Inc.',
    workspace_slug = 'virivu-platform'
WHERE organization_kind = 'platform_root';

UPDATE data_use_agreements
SET counterparty_name = 'Virivu Research Cloud Services, Inc.'
WHERE counterparty_name = 'Cingulum Foundation Inc.';

ALTER TABLE data_use_agreements
    ALTER COLUMN counterparty_name SET DEFAULT 'Virivu Research Cloud Services, Inc.';

UPDATE data_use_agreement_signatures
SET signer_organization = 'Virivu Research Cloud Services, Inc.'
WHERE signer_role = 'cingulum'
  AND signer_organization = 'Cingulum Foundation Inc.';
