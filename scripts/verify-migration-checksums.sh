#!/usr/bin/env bash
# Fail if any file in backend/migrations differs from DB _sqlx_migrations (breaks api/worker migrate).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MIG_DIR="$ROOT/backend/migrations"
URL="${MIGRATION_DATABASE_URL:-postgres://pommarket:${POSTGRES_PASSWORD:-pommarket_dev_password}@127.0.0.1:5440/pommarket}"

if ! command -v psql >/dev/null; then
  echo "psql required" >&2
  exit 1
fi

failed=0
while IFS= read -r file; do
  ver=$(basename "$file" | sed -E 's/^0*([0-9]+).*/\1/')
  file_sum=$(sha384sum "$file" | awk '{print $1}')
  db_sum=$(psql "$URL" -tAc "SELECT encode(checksum, 'hex') FROM _sqlx_migrations WHERE version = $ver" 2>/dev/null || true)
  db_sum=$(echo "$db_sum" | tr -d '[:space:]')
  if [ -z "$db_sum" ]; then
    echo "migration $ver: not applied in DB (ok for new files)"
    continue
  fi
  if [ "$file_sum" != "$db_sum" ]; then
    echo "MISMATCH migration $ver: file edited after apply — api/worker will refuse to start" >&2
    failed=1
  fi
done < <(ls -1 "$MIG_DIR"/*.sql 2>/dev/null | sort)

if [ "$failed" -ne 0 ]; then
  exit 1
fi
echo "All applied migrations match files."
