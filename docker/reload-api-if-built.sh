#!/bin/sh
# Called by cargo-watch on file changes. Rebuilds and swaps pom-api only on success.
set -eu

CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/src/target-api}"
BIN="$CARGO_TARGET_DIR/debug/pom-api"
PIDFILE="${PIDFILE:-/tmp/pom-api-dev.pid}"

if ! cargo build --bin pom-api >&2; then
  echo "[reload-api] build failed; keeping the previous API process" >&2
  exit 0
fi

if [ -f "$PIDFILE" ]; then
  old=$(cat "$PIDFILE")
  kill "$old" 2>/dev/null || true
  wait "$old" 2>/dev/null || true
fi

"$BIN" &
echo $! >"$PIDFILE"
