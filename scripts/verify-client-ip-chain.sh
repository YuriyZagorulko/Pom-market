#!/usr/bin/env bash
# Empirical check: Browser → User Next (:4323) → API with spoofed X-Forwarded-For.
# Expect: with empty TRUSTED_PROXIES, a new XFF value cannot bypass a filled login bucket.
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
USER_ORIGIN="${USER_ORIGIN:-http://127.0.0.1:4323}"
EMAIL="chain-check-$(date +%s)@example.com"

status() {
  local xff=$1
  curl -s -o /dev/null -w "%{http_code}" -X POST "${USER_ORIGIN}/api/v1/auth/login" \
    -H "Content-Type: application/json" \
    -H "X-Forwarded-For: ${xff}" \
    -d "{\"email\":\"${EMAIL}\",\"password\":\"wrong\"}"
}

echo "Using USER_ORIGIN=${USER_ORIGIN} (ensure API TRUSTED_PROXIES is empty)"
for i in $(seq 1 10); do
  code=$(status "192.0.2.44")
  echo "attempt $i xff=192.0.2.44 => HTTP $code"
  [[ "$code" == "401" ]] || { echo "unexpected"; exit 1; }
done
code=$(status "192.0.2.44")
echo "11th same xff => HTTP $code (expect 429)"
[[ "$code" == "429" ]] || exit 1
code=$(status "192.0.2.45")
echo "12th new xff => HTTP $code (expect 429 if middleware + API trust model OK)"
[[ "$code" == "429" ]] || exit 1
echo "PASS: spoofed X-Forwarded-For does not bypass login limiter via Next proxy"
