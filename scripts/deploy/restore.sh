#!/usr/bin/env bash
# Restore a pg_dump custom-format backup into postgres (DESTRUCTIVE on target DB).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.coolify.yml}"
ENV_FILE="${ENV_FILE:-.env}"
DUMP="${1:-}"

[[ -n "$DUMP" && -f "$DUMP" ]] || {
  echo "usage: CONFIRM_RESTORE=YES $0 backups/pommarket-YYYYMMDD.dump" >&2
  exit 1
}

[[ "${CONFIRM_RESTORE:-}" == "YES" ]] || {
  echo "restore: refused — set CONFIRM_RESTORE=YES to overwrite database pommarket" >&2
  exit 1
}

[[ -f "$ENV_FILE" ]] || { echo "restore: missing $ENV_FILE" >&2; exit 1; }

echo "restore: stopping api and worker..."
docker compose -f "$COMPOSE_FILE" --env-file "$ENV_FILE" stop api worker 2>/dev/null || true

docker compose -f "$COMPOSE_FILE" --env-file "$ENV_FILE" up -d postgres

docker compose -f "$COMPOSE_FILE" --env-file "$ENV_FILE" cp "$DUMP" \
  "postgres:/tmp/restore.dump"

docker compose -f "$COMPOSE_FILE" --env-file "$ENV_FILE" exec -T postgres \
  pg_restore -U pommarket -d pommarket --clean --if-exists --no-owner /tmp/restore.dump

docker compose -f "$COMPOSE_FILE" --env-file "$ENV_FILE" exec -T postgres \
  rm -f /tmp/restore.dump

echo "restore: finished — start api/worker manually"