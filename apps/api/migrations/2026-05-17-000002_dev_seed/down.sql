DELETE FROM user_memberships
WHERE role = 'platform_admin'
  AND user_id IN (
    SELECT id
    FROM users
    WHERE email IN ('admin@cingulum.org', 'arcot@cingulum.org')
  );

DELETE FROM users
WHERE email IN ('admin@cingulum.org', 'arcot@cingulum.org');
