#!/usr/bin/env bash
# Inject a mock source trade into the running primary DB (PAPER pipeline only).
# Usage: INJECT_WALLET=0x... INJECT_SIDE=buy INJECT_TX_SUFFIX=unique1 ./scripts/inject-paper-event.sh
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
set -a
# shellcheck disable=SC1091
source .env
set +a
: "${INJECT_WALLET:?set INJECT_WALLET to the monitored source wallet}"
: "${INJECT_TX_SUFFIX:?set INJECT_TX_SUFFIX to a unique suffix}"
INJECT_SIDE="${INJECT_SIDE:-buy}"
export POSTGRES_PORT="${POSTGRES_PORT:-5440}"
if [[ "${E2E_MODE:-}" == "1" ]]; then
  export E2E_DATABASE_OK=1
  export WORKER_DATABASE_URL="postgres://pommarket_worker:${WORKER_DB_PASSWORD}@127.0.0.1:${POSTGRES_PORT}/pommarket_e2e"
else
  export E2E_PRIMARY_OK=1
  export WORKER_DATABASE_URL="postgres://pommarket_worker:${WORKER_DB_PASSWORD}@127.0.0.1:${POSTGRES_PORT}/pommarket"
fi
export INJECT_WALLET INJECT_SIDE INJECT_TX_SUFFIX
export POLYMARKET_CLOB_HOST="${POLYMARKET_CLOB_HOST:-https://clob.polymarket.com}"
cargo test --manifest-path backend/Cargo.toml -p pommarket \
  --test inject_paper_event inject_trade -- --ignored --nocapture
