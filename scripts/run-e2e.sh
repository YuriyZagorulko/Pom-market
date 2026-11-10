#!/usr/bin/env bash
# Playwright E2E against isolated pommarket_e2e (profile e2e). Does not touch pommarket.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
set -a
# shellcheck disable=SC1091
source .env
set +a

export POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-pommarket_dev_password}"
export POSTGRES_PORT="${POSTGRES_PORT:-5440}"
export E2E_API_PORT="${E2E_API_PORT:-4324}"
export E2E_ADMIN_PORT="${E2E_ADMIN_PORT:-4325}"
export E2E_USER_PORT="${E2E_USER_PORT:-4326}"

bash scripts/ensure-test-databases.sh

count_primary_users() {
  docker compose exec -T postgres psql -U pommarket -d pommarket -tAc "SELECT count(*) FROM users"
}

before_users=$(count_primary_users)
echo "pommarket users before E2E: ${before_users}"

docker compose -f docker-compose.yml -f docker-compose.e2e.yml --profile e2e up -d --build api-e2e worker-e2e admin-e2e user-e2e
for _ in $(seq 1 60); do
  if curl -fsS "http://127.0.0.1:${E2E_API_PORT}/health" >/dev/null 2>&1; then
    break
  fi
  sleep 2
done

db_name=$(curl -fsS "http://127.0.0.1:${E2E_API_PORT}/health" | sed -n 's/.*"database_name":"\([^"]*\)".*/\1/p')
if [[ "$db_name" != "pommarket_e2e" ]]; then
  echo "refusing E2E: api-e2e health database_name=${db_name:-missing}, expected pommarket_e2e" >&2
  exit 1
fi

mkdir -p e2e
cat > e2e/.env.local <<EOF
E2E_ADMIN_BASE_URL=http://127.0.0.1:${E2E_ADMIN_PORT}
E2E_USER_BASE_URL=http://127.0.0.1:${E2E_USER_PORT}
E2E_API_BASE_URL=http://127.0.0.1:${E2E_API_PORT}
E2E_ADMIN_EMAIL=${BOOTSTRAP_ADMIN_EMAIL}
E2E_ADMIN_PASSWORD=${BOOTSTRAP_ADMIN_PASSWORD}
E2E_INVITED_USER_PASSWORD=${E2E_INVITED_USER_PASSWORD:-playwright-invite-10}
E2E_DATABASE_NAME=pommarket_e2e
E2E_USE_ISOLATED_DB=1
EOF

if command -v pnpm >/dev/null 2>&1; then
  PNPM=pnpm
else
  PNPM="npx --yes pnpm@9.15.9"
fi
$PNPM install
export E2E_MODE=1
export WORKER_DATABASE_URL="postgres://pommarket_worker:${WORKER_DB_PASSWORD}@127.0.0.1:${POSTGRES_PORT}/pommarket_e2e"
if [ "${E2E_IN_DOCKER:-}" = "1" ]; then
  $PNPM --filter @pom/e2e test
else
  docker run --rm --network host \
    -v "$root:/app" -w /app \
    -e E2E_IN_DOCKER=1 \
    -e E2E_MODE=1 \
    -e WORKER_DATABASE_URL \
    mcr.microsoft.com/playwright:v1.49.1-jammy \
    bash -lc 'corepack enable && npx -y pnpm@9.15.9 install --force && npx -y pnpm@9.15.9 --filter @pom/e2e exec playwright test'
fi

after_users=$(count_primary_users)
echo "pommarket users after E2E: ${after_users}"
if [[ "$before_users" != "$after_users" ]]; then
  echo "E2E isolation FAILED: pommarket user count changed ${before_users} -> ${after_users}" >&2
  exit 1
fi

e2e_users=$(docker compose exec -T postgres psql -U pommarket -d pommarket_e2e -tAc "SELECT count(*) FROM users")
echo "pommarket_e2e users after E2E: ${e2e_users}"
