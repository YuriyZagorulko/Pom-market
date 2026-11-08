#!/usr/bin/env bash
set -euo pipefail

# PostgreSQL Backup & Point-in-Time Recovery helper for Pom-Market
# Usage:
#   ./deploy/scripts/backup_and_restore.sh backup [output_file.dump]
#   ./deploy/scripts/backup_and_restore.sh restore <input_file.dump>

POSTGRES_PORT="${POSTGRES_PORT:-5440}"
DB_URL="${DATABASE_URL:-postgresql://pommarket:pommarket_dev_pass@localhost:${POSTGRES_PORT}/pommarket}"
ACTION="${1:-backup}"

if [[ "$ACTION" == "backup" ]]; then
  OUT_FILE="${2:-pommarket_backup_$(date -u +%Y%m%dT%H%M%SZ).dump}"
  echo "[pom-market] Creating compressed PostgreSQL backup -> ${OUT_FILE}"
  pg_dump --format=custom --no-owner --no-privileges "$DB_URL" --file="$OUT_FILE"
  echo "[pom-market] Backup completed successfully."
elif [[ "$ACTION" == "restore" ]]; then
  IN_FILE="${2:?Error: restore requires an input .dump file path}"
  echo "[pom-market] Restoring PostgreSQL database from ${IN_FILE}"
  pg_restore --clean --if-exists --no-owner --no-privileges --dbname="$DB_URL" "$IN_FILE"
  echo "[pom-market] Restore completed. Run reconciliation service before resuming LIVE_TRADING."
else
  echo "Usage: $0 {backup [file]|restore <file>}"
  exit 1
fi
