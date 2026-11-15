#!/usr/bin/env bash
# Validate Coolify/staging deployment configuration before build or deploy.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.prod.yml}"
ENV_FILE="${ENV_FILE:-.env}"

fail() {
  echo "validate: $*" >&2
  exit 1
}

[[ -f "$COMPOSE_FILE" ]] || fail "missing $COMPOSE_FILE"
command -v docker >/dev/null || fail "docker not found"

compose_config() {
  local args=(--env-file "$ENV_FILE" -f "$COMPOSE_FILE")
  if [[ -n "${COMPOSE_EXTRA_FILES:-}" ]]; then
    local f
    for f in ${COMPOSE_EXTRA_FILES}; do args+=(-f "$f"); done
  fi
  if [[ -n "${COMPOSE_PROJECT_NAME:-}" ]]; then
    docker compose -p "$COMPOSE_PROJECT_NAME" "${args[@]}" config >/dev/null
  else
    docker compose "${args[@]}" config >/dev/null
  fi
}

compose_config

if [[ -f "$ENV_FILE" ]]; then
  set -a
  # shellcheck disable=SC1090
  source "$ENV_FILE"
  set +a
else
  fail "missing $ENV_FILE (copy from .env.production.example)"
fi

for var in \
  MIGRATION_DATABASE_URL APP_DATABASE_URL WORKER_DATABASE_URL \
  JWT_SECRET ENCRYPTION_MASTER_KEY USER_ORIGIN APP_BASE_URL ADMIN_ORIGIN; do
  [[ -n "${!var:-}" ]] || fail "unset $var"
done

reject_deprecated_origin() {
  local name=$1 value=$2
  [[ "$value" != *"admin.pommarket.pomvix.com"* ]] \
    || fail "$name uses deprecated host admin.pommarket.pomvix.com (use pommarket-admin.pomvix.com)"
  [[ "$value" != *"api.pommarket.pomvix.com"* ]] \
    || fail "$name uses deprecated host api.pommarket.pomvix.com (use pommarket-api.pomvix.com)"
}

for var in USER_ORIGIN APP_BASE_URL ADMIN_ORIGIN PUBLIC_API_ORIGIN CORS_ORIGINS; do
  v="${!var:-}"
  [[ -z "$v" ]] && continue
  reject_deprecated_origin "$var" "$v"
done

[[ "${CONFIRM_LIVE_TRADING:-false}" == "false" ]] \
  || fail "CONFIRM_LIVE_TRADING must be false for deployment prep"

case "${ENVIRONMENT:-staging}" in
  production)
  for var in JWT_SECRET ENCRYPTION_MASTER_KEY; do
    v="${!var}"
    [[ ${#v} -ge 32 ]] || fail "$var too short for production"
    [[ "$v" != *CHANGE_ME* ]] || fail "$var still contains CHANGE_ME"
  done
  [[ -n "${PUBLIC_API_ORIGIN:-}" ]] || fail "unset PUBLIC_API_ORIGIN (required in production)"
  reject_deprecated_origin PUBLIC_API_ORIGIN "${PUBLIC_API_ORIGIN}"
  api_internal="${API_INTERNAL_URL:-http://api:4321}"
  [[ "$api_internal" == http://api:* ]] \
    || fail "API_INTERNAL_URL must stay on Docker DNS (e.g. http://api:4321), got: $api_internal"
  ;;
esac

for url in MIGRATION_DATABASE_URL APP_DATABASE_URL WORKER_DATABASE_URL; do
  u="${!url}"
  [[ "$u" != *CHANGE_ME* ]] || fail "$url contains CHANGE_ME"
  [[ "$u" != *pommarket_dev_password* ]] || fail "$url contains dev password"
done

bash scripts/verify-migration-checksums.sh

echo "validate: OK ($COMPOSE_FILE, $ENV_FILE)"