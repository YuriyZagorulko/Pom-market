#!/usr/bin/env bash
set -euo pipefail
: "${MIGRATION_DATABASE_URL:?set MIGRATION_DATABASE_URL}"
dump=${1:?usage: restore.sh backups/file.dump}
pg_restore --clean --if-exists --no-owner --dbname "$MIGRATION_DATABASE_URL" "$dump"
