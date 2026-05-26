ALTER TABLE providers
    DROP COLUMN IF EXISTS email,
    DROP COLUMN IF EXISTS phone_number,
    DROP COLUMN IF EXISTS npi_number,
    DROP COLUMN IF EXISTS address,
    DROP COLUMN IF EXISTS notes;
