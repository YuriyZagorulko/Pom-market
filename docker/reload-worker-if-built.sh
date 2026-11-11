#!/bin/sh
set -eu

CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/src/target-worker}"
BIN="$CARGO_TARGET_DIR/debug/pom-worker"
PIDFILE="${PIDFILE:-/tmp/pom-worker-dev.pid}"

if ! cargo build --bin pom-worker >&2; then
  echo "[reload-worker] build failed; keeping the previous worker process" >&2
  exit 0
fi

if [ -f "$PIDFILE" ]; then
  old=$(cat "$PIDFILE")
  kill "$old" 2>/dev/null || true
  wait "$old" 2>/dev/null || true
fi

"$BIN" &
echo $! >"$PIDFILE"
