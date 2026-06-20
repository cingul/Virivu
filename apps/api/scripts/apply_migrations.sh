#!/usr/bin/env bash
set -euo pipefail

if [[ -z "${DATABASE_URL:-}" ]]; then
  echo "DATABASE_URL must be set"
  exit 1
fi

echo "Running Diesel migrations..."
cargo run --quiet --bin migrate
echo "Diesel migrations completed successfully."
