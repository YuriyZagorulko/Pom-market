#!/usr/bin/env bash
# Create pommarket_test (integration) and pommarket_e2e (Playwright) if missing.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
export POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-pommarket_dev_password}"
export POSTGRES_PORT="${POSTGRES_PORT:-5440}"
docker compose up -d postgres
for _ in $(seq 1 30); do
  if docker compose exec -T postgres pg_isready -U pommarket -d pommarket >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
create_db() {
  local name=$1
  docker compose exec -T postgres psql -U pommarket -d postgres -tc \
    "SELECT 1 FROM pg_database WHERE datname = '${name}'" | grep -q 1 \
    || docker compose exec -T postgres psql -U pommarket -d postgres -c \
      "CREATE DATABASE ${name} OWNER pommarket"
  docker compose exec -T postgres psql -U pommarket -d postgres -c \
    "GRANT ALL PRIVILEGES ON DATABASE ${name} TO pommarket"
}
create_db pommarket_test
create_db pommarket_e2e
