#!/usr/bin/env bash
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
export POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-pommarket_dev_password}"
export POSTGRES_PORT="${POSTGRES_PORT:-5440}"
docker compose up -d postgres
bash scripts/ensure-test-databases.sh
for _ in $(seq 1 30); do
  if docker compose exec -T postgres pg_isready -U pommarket -d pommarket >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
docker compose exec -T postgres psql -U pommarket -d postgres -c "SELECT 1 FROM pg_database WHERE datname = 'pommarket_test'" | grep -q 1 \
  || docker compose exec -T postgres psql -U pommarket -d postgres -c "CREATE DATABASE pommarket_test"
export APP_DB_PASSWORD="${APP_DB_PASSWORD:-app_dev_password}"
export WORKER_DB_PASSWORD="${WORKER_DB_PASSWORD:-worker_dev_password}"
export MIGRATION_DATABASE_URL="postgres://pommarket:${POSTGRES_PASSWORD}@localhost:${POSTGRES_PORT}/pommarket_test"
export APP_DATABASE_URL="postgres://pommarket_app:${APP_DB_PASSWORD}@localhost:${POSTGRES_PORT}/pommarket_test"
export WORKER_DATABASE_URL="postgres://pommarket_worker:${WORKER_DB_PASSWORD}@localhost:${POSTGRES_PORT}/pommarket_test"
export JWT_SECRET="${JWT_SECRET:-integration-test-jwt-secret-0123456789abcdef}"
export ENCRYPTION_MASTER_KEY="${ENCRYPTION_MASTER_KEY:-integration-test-master-key-32-chars-min}"
export ENCRYPTION_KEY_ID="${ENCRYPTION_KEY_ID:-k1}"
export BOOTSTRAP_ADMIN_EMAIL=admin@example.com
export BOOTSTRAP_ADMIN_PASSWORD=admin-password-1
export BOOTSTRAP_ADMIN_NAME=Admin
export CONFIRM_LIVE_TRADING=false
export ENVIRONMENT=test
export EMAIL_MODE=mock
export E2E_MODE=0
unset BREVO_API_KEY BREEVO_API_KEY BREVO_API_BASE 2>/dev/null || true
# Local test client connects from 127.0.0.0/8; only then may X-Forwarded-For affect buckets.
export TRUSTED_PROXIES="${TRUSTED_PROXIES:-127.0.0.0/8}"
cargo test --manifest-path backend/Cargo.toml \
  --test ingest --test flow --test email_auth --test password_reset --test email_transport_safety \
  --test orders_filter --test pipeline_scenarios --test rate_limit_login \
  --test copy_trading_workflow \
  --test copy_trading_concurrency \
  --test copy_trading_attribution \
  --test account_trading_control \
  --test hard_close_safety \
  -- --ignored --test-threads=1 --nocapture
cargo test --manifest-path backend/Cargo.toml --test email_transport_safety -- --nocapture
