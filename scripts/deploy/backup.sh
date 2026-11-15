#!/usr/bin/env bash
# Backup PostgreSQL via docker compose exec (no secrets printed).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.coolify.yml}"
ENV_FILE="${ENV_FILE:-.env}"
OUT_DIR="${OUT_DIR:-backups}"

[[ -f "$ENV_FILE" ]] || { echo "backup: missing $ENV_FILE" >&2; exit 1; }
mkdir -p "$OUT_DIR"

stamp="$(date -u +%Y%m%dT%H%M%SZ)"
out="$OUT_DIR/pommarket-${stamp}.dump"

docker compose -f "$COMPOSE_FILE" --env-file "$ENV_FILE" exec -T postgres \
  pg_dump -U pommarket -d pommarket -Fc --no-owner -f "/tmp/pommarket-${stamp}.dump"

docker compose -f "$COMPOSE_FILE" --env-file "$ENV_FILE" cp \
  "postgres:/tmp/pommarket-${stamp}.dump" "$out"

docker compose -f "$COMPOSE_FILE" --env-file "$ENV_FILE" exec -T postgres \
  rm -f "/tmp/pommarket-${stamp}.dump"

echo "backup: wrote $out"