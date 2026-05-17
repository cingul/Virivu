#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${DATABASE_URL:-}" ]]; then
  echo "DATABASE_URL must be set"
  exit 1
fi

psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 -c "
CREATE TABLE IF NOT EXISTS schema_migrations (
  name TEXT PRIMARY KEY,
  applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
"

for file in migrations/*.sql; do
  migration_name="$(basename "$file")"
  migration_name_escaped="${migration_name//\'/\'\'}"
  already_applied="$(
    psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 -At \
      -c "SELECT 1 FROM schema_migrations WHERE name = '${migration_name_escaped}' LIMIT 1;"
  )"

  if [[ "$already_applied" == "1" ]]; then
    echo "Skipping $migration_name (already applied)"
    continue
  fi

  echo "Applying $migration_name"
  psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 -f "$file"
  psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 \
    -c "INSERT INTO schema_migrations(name) VALUES ('${migration_name_escaped}');"
done

echo "Migrations applied successfully."
