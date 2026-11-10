#!/bin/sh
# Dev-only: stale .next chunks (often from bind-mount copy-up into an anonymous volume)
# cause "Cannot find module './NNN.js'" in next dev. Wipe before start unless opted out.
set -e
next_dir="${NEXT_DEV_NEXT_DIR:-}"
if [ -n "$next_dir" ] && [ "${NEXT_DEV_ALWAYS_CLEAN:-1}" != "0" ]; then
  mkdir -p "$next_dir"
  # .next is often a volume mount; clearing contents avoids "Device or resource busy".
  find "$next_dir" -mindepth 1 -delete 2>/dev/null || true
fi
exec "$@"
