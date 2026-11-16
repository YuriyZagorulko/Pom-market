#!/usr/bin/env bash
# Ensure SQL migration files match what Rust binaries embed at compile time.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MIG_DIR="$ROOT/backend/migrations"

count=$(find "$MIG_DIR" -maxdepth 1 -name '*.sql' | wc -l)
head=$(find "$MIG_DIR" -maxdepth 1 -name '*.sql' -printf '%f\n' | sed -E 's/^0*([0-9]+).*/\1/' | sort -n | tail -1)

if [[ -z "$head" || "$head" -lt 1 ]]; then
  echo "verify-embedded-migrations: no migration files in $MIG_DIR" >&2
  exit 1
fi

if [[ "$count" -ne "$head" ]]; then
  echo "verify-embedded-migrations: file count ($count) != highest version ($head) — check for gaps" >&2
  exit 1
fi

cd "$ROOT/backend"
cargo test -p pommarket --lib migration_source_tests -- --nocapture

echo "verify-embedded-migrations: OK (head migration v$head, $count files)"