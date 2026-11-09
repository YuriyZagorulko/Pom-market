#!/usr/bin/env bash
set -euo pipefail
echo "POSTGRES_PASSWORD=$(openssl rand -hex 24)"
echo "APP_DB_PASSWORD=$(openssl rand -hex 24)"
echo "WORKER_DB_PASSWORD=$(openssl rand -hex 24)"
echo "JWT_SECRET=$(openssl rand -hex 32)"
echo "ENCRYPTION_MASTER_KEY=$(openssl rand -base64 32)"
echo "BOOTSTRAP_ADMIN_PASSWORD=$(openssl rand -base64 18)"
