#!/bin/bash
# One-shot check that nginx -> PHP login.php accepts the benchmark payload.
#   bash login-server/bench/check-php.sh
#   bash login-server/bench/check-php.sh http://127.0.0.1:8082/login.php
set -euo pipefail

URL="${1:-http://127.0.0.1:8082/login.php}"
HDR="$(mktemp)"
BODY="$(mktemp)"
trap 'rm -f "$HDR" "$BODY"' EXIT

post() {
  local name="$1"
  local payload="$2"
  local code
  code="$(curl -sS -m 30 -D "$HDR" -o "$BODY" -w '%{http_code}' \
    --http1.1 \
    -X POST "$URL" \
    -H 'Content-Type: application/json' \
    -H 'Connection: keep-alive' \
    -d "$payload")"
  echo
  echo "===== $name ====="
  echo "http_code $code"
  echo "--- headers ---"
  cat "$HDR"
  echo "--- body ---"
  cat "$BODY"
  echo
}

echo "url $URL"
post hit '{"type":"login","accountname":"bench00001","password":"secret"}'
if ! grep -q 'sessionkey' "$BODY"; then
  echo "FAIL: hit response has no sessionkey" >&2
  exit 1
fi
if grep -qi 'transfer-encoding: chunked' "$HDR"; then
  echo "note: PHP response is chunked; login-bench must be rebuilt to accept that"
fi

post bad '{"type":"login","accountname":"bench00001","password":"wrong"}'
if grep -q 'sessionkey' "$BODY"; then
  echo "FAIL: wrong password returned a session" >&2
  exit 1
fi
if ! grep -q 'errorMessage' "$BODY"; then
  echo "FAIL: wrong password has no errorMessage" >&2
  exit 1
fi

post miss '{"type":"login","accountname":"miss00001","password":"secret"}'
if grep -q 'sessionkey' "$BODY"; then
  echo "FAIL: missing account returned a session" >&2
  exit 1
fi
if ! grep -q 'errorMessage' "$BODY"; then
  echo "FAIL: missing account has no errorMessage" >&2
  exit 1
fi

echo
echo "ok: PHP login hit, bad password and missing account"
