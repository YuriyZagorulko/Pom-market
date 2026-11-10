#!/usr/bin/env bash
set -euo pipefail
export MIGRATION_DATABASE_URL=postgres://pommarket:pommarket_dev_password@localhost:5440/pommarket_test
export APP_DATABASE_URL=postgres://pommarket_app:app_dev_password@localhost:5440/pommarket_test
export WORKER_DATABASE_URL=postgres://pommarket_worker:worker_dev_password@localhost:5440/pommarket_test
export APP_DB_PASSWORD=app_dev_password
export WORKER_DB_PASSWORD=worker_dev_password
export ENCRYPTION_MASTER_KEY=integration-test-master-key-32-chars-min
export JWT_SECRET=integration-test-jwt-secret-0123456789abcdef
export BOOTSTRAP_ADMIN_EMAIL=admin@example.com
export BOOTSTRAP_ADMIN_PASSWORD=admin-password-1
export ENVIRONMENT=test
export EMAIL_MODE=mock
unset BREVO_API_KEY BREEVO_API_KEY BREVO_API_BASE 2>/dev/null || true
export RUST_LOG=info
cargo test --manifest-path backend/Cargo.toml --test ingest concurrent_dedup_ingestion_is_safe -- --ignored --nocapture "$@"
