-- Optional local-development seed data.
-- Adjust emails/domain to match GOOGLE_WORKSPACE_DOMAIN before running.

INSERT INTO users (email, google_subject, display_name)
VALUES ('admin@cingulum.org', 'dev-admin@cingulum.org', 'Virivu Admin')
ON CONFLICT (email)
DO UPDATE SET display_name = EXCLUDED.display_name;

INSERT INTO user_memberships (user_id, role, status)
SELECT u.id, 'platform_admin', 'active'
FROM users u
WHERE u.email = 'admin@cingulum.org'
  AND NOT EXISTS (
      SELECT 1
      FROM user_memberships um
      WHERE um.user_id = u.id
        AND um.role = 'platform_admin'
        AND um.organization_id IS NULL
        AND um.project_id IS NULL
  );
