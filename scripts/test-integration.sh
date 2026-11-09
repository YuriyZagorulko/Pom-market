#!/usr/bin/env bash
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
docker compose exec -T postgres psql -U pommarket -d postgres -c "SELECT 1 FROM pg_database WHERE datname = 'pommarket_test'" | grep -q 1 \
  || docker compose exec -T postgres psql -U pommarket -d postgres -c "CREATE DATABASE pommarket_test"
export APP_DB_PASSWORD="${APP_DB_PASSWORD:-app_test_password}"
export WORKER_DB_PASSWORD="${WORKER_DB_PASSWORD:-worker_test_password}"
export MIGRATION_DATABASE_URL="postgres://pommarket:${POSTGRES_PASSWORD}@localhost:${POSTGRES_PORT}/pommarket_test"
export APP_DATABASE_URL="postgres://pommarket_app:${APP_DB_PASSWORD}@localhost:${POSTGRES_PORT}/pommarket_test"
export WORKER_DATABASE_URL="postgres://pommarket_worker:${WORKER_DB_PASSWORD}@localhost:${POSTGRES_PORT}/pommarket_test"
export JWT_SECRET="${JWT_SECRET:-integration-test-jwt-secret-0123456789abcdef}"
export ENCRYPTION_MASTER_KEY="${ENCRYPTION_MASTER_KEY:-integration-test-master-key}"
export ENCRYPTION_KEY_ID="${ENCRYPTION_KEY_ID:-k1}"
export BOOTSTRAP_ADMIN_EMAIL=admin@example.com
export BOOTSTRAP_ADMIN_PASSWORD=admin-password-1
export BOOTSTRAP_ADMIN_NAME=Admin
export CONFIRM_LIVE_TRADING=false
cargo test --manifest-path backend/Cargo.toml --test flow -- --ignored --nocapture
