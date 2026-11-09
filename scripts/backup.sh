#!/usr/bin/env bash
set -euo pipefail
: "${MIGRATION_DATABASE_URL:?set MIGRATION_DATABASE_URL}"
mkdir -p backups
stamp=$(date -u +%Y%m%dT%H%M%SZ)
pg_dump --format=custom --no-owner --file "backups/pommarket-${stamp}.dump" "$MIGRATION_DATABASE_URL"
echo "wrote backups/pommarket-${stamp}.dump"
