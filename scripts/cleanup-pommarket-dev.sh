#!/usr/bin/env bash
# Remove test users from primary local DB pommarket; keep allowlisted accounts only.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
set -a
# shellcheck disable=SC1091
source .env
set +a

export POSTGRES_PASSWORD="${POSTGRES_PASSWORD:-pommarket_dev_password}"

KEEP=(
  "cc83308f-a64e-45a6-b96a-d342b6ae7ba4"
  "6f18a3a5-bfde-4178-96d7-950733fbefa5"
  "4e2130d5-27ed-47f6-8e84-74572549c147"
)

psql_exec() {
  docker compose exec -T postgres psql -U pommarket -d pommarket -v ON_ERROR_STOP=1 "$@"
}

current_db=$(docker compose exec -T postgres psql -U pommarket -d pommarket -tAc "SELECT current_database()")
if [[ "$current_db" != "pommarket" ]]; then
  echo "refusing: connected to ${current_db}, expected pommarket" >&2
  exit 1
fi

for id in "${KEEP[@]}"; do
  psql_exec -c "SELECT 1 FROM users WHERE id = '${id}'::uuid" | grep -q 1 \
    || { echo "missing allowlisted user ${id}" >&2; exit 1; }
done

echo "users before: $(psql_exec -tAc 'SELECT count(*) FROM users')"

in_list=$(printf "'%s'," "${KEEP[@]}")
in_list="${in_list%,}"

psql_exec <<SQL
BEGIN;
DELETE FROM users WHERE id NOT IN (${in_list});
DELETE FROM source_events
  WHERE id NOT IN (SELECT source_event_id FROM copy_decisions);
COMMIT;
SQL

echo "users after: $(psql_exec -tAc 'SELECT count(*) FROM users')"
psql_exec -c "SELECT id, email, role, status FROM users ORDER BY email;"
