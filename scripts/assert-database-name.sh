#!/usr/bin/env bash
# Assert PostgreSQL database name in a connection URL (no secrets logged).
# Usage: ./scripts/assert-database-name.sh EXPECTED_NAME "$DATABASE_URL"
set -euo pipefail
expected="${1:?expected database name}"
url="${2:?database URL}"
actual="${url##*/}"
actual="${actual%%\?*}"
if [[ "$actual" != "$expected" ]]; then
  echo "refusing: expected database '${expected}', URL targets '${actual}'" >&2
  exit 1
fi
