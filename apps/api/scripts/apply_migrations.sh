#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${DATABASE_URL:-}" ]]; then
  echo "DATABASE_URL must be set"
  exit 1
fi

for file in migrations/*.sql; do
  echo "Applying $file"
  psql "${DATABASE_URL}" -v ON_ERROR_STOP=1 -f "$file"
done

echo "Migrations applied successfully."
