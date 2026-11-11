#!/bin/sh
# Dev-only: stale .next chunks (often from bind-mount copy-up into an anonymous volume)
# cause "Cannot find module './NNN.js'" in next dev. Wipe before start unless opted out.
set -e
next_dir="${NEXT_DEV_NEXT_DIR:-}"
if [ -n "$next_dir" ]; then
  mkdir -p "$next_dir"
  clean=0
  if [ "${NEXT_DEV_ALWAYS_CLEAN:-1}" != "0" ]; then
    clean=1
  elif [ -f "$next_dir/required-server-files.json" ] || [ -d "$next_dir/export" ]; then
    # next build inside the dev container leaves prod artifacts; next dev then 404s /_next/static/chunks/*.js
    clean=1
  fi
  if [ "$clean" = 1 ]; then
    find "$next_dir" -mindepth 1 -delete 2>/dev/null || true
  fi
fi
exec "$@"
