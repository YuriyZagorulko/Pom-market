#!/usr/bin/env bash
# Apply SQLx migrations once (bootstrap roles + admin). Does not start HTTP server.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.prod.yml}"
ENV_FILE="${ENV_FILE:-.env}"

[[ -f "$ENV_FILE" ]] || { echo "migrate: missing $ENV_FILE" >&2; exit 1; }

set -a
# shellcheck disable=SC1090
source "$ENV_FILE"
set +a

compose() {
  local args=(--env-file "$ENV_FILE" -f "$COMPOSE_FILE")
  if [[ -n "${COMPOSE_EXTRA_FILES:-}" ]]; then
    local f
    for f in ${COMPOSE_EXTRA_FILES}; do args+=(-f "$f"); done
  fi
  if [[ -n "${COMPOSE_PROJECT_NAME:-}" ]]; then
    docker compose -p "$COMPOSE_PROJECT_NAME" "${args[@]}" "$@"
  else
    docker compose "${args[@]}" "$@"
  fi
}

compose up -d postgres

echo "migrate: waiting for postgres..."
compose exec -T postgres \
  sh -c 'until pg_isready -U pommarket -d pommarket; do sleep 1; done'

echo "migrate: running pom-api --migrate-only..."
compose run --rm --no-deps migrate

echo "migrate: done"