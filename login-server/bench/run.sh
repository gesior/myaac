#!/bin/bash
# Login benchmarks. Services stay on CPUs 0-3; this process is pinned to 4-15.
#
#   bash login-server/bench/run.sh
#   PROFILE=hot bash login-server/bench/run.sh
#
# spread: each hit walks every seeded account (default 10000).
# hot: hit repeats bench00001..bench00100. The database should hold 50000
# accounts (bash login-server/bench/reseed.sh 50000) so the account list is
# large while character lists for those 100 stay cached.
set -euo pipefail

BENCH="${BENCH:-/usr/local/bin/login-bench}"
if [[ ! -x "$BENCH" ]]; then
  BENCH="$(cd "$(dirname "$0")" && pwd)/target/release/login-bench"
fi
TASKSET_CPUS="${TASKSET_CPUS:-4-15}"
PROFILE="${PROFILE:-spread}"
case "$PROFILE" in
  spread)
    HIT_ACCOUNTS="${HIT_ACCOUNTS:-${ACCOUNTS:-10000}}"
    BAD_ACCOUNTS="${BAD_ACCOUNTS:-${ACCOUNTS:-10000}}"
    MISS_ACCOUNTS="${MISS_ACCOUNTS:-${ACCOUNTS:-10000}}"
    ;;
  hot)
    HIT_ACCOUNTS="${HIT_ACCOUNTS:-100}"
    BAD_ACCOUNTS="${BAD_ACCOUNTS:-50000}"
    MISS_ACCOUNTS="${MISS_ACCOUNTS:-50000}"
    ;;
  *)
    echo "unknown profile $PROFILE (use spread or hot)" >&2
    exit 1
    ;;
esac
WARMUP="${WARMUP:-10}"
DURATION="${DURATION:-30}"
THREADS="${THREADS:-12}"
CONNECTIONS="${CONNECTIONS:-256}"
RUST_URL="${RUST_URL:-http://127.0.0.1:8080/login.php}"
PHP_URL="${PHP_URL:-http://127.0.0.1:8082/login.php}"
HEALTH_URL="${HEALTH_URL:-http://127.0.0.1:8081/health}"

ulimit -n 1048576 || true

echo "profile: ${PROFILE}"
echo "hit_accounts: ${HIT_ACCOUNTS}"
echo "bad_accounts: ${BAD_ACCOUNTS}"
echo "miss_accounts: ${MISS_ACCOUNTS}"

if [[ "$PROFILE" == "hot" ]]; then
  loaded="$(curl -sf "$HEALTH_URL" || true)"
  loaded_accounts="$(printf '%s' "$loaded" | sed -n 's/.*"accounts":\([0-9][0-9]*\).*/\1/p')"
  if [[ -z "$loaded_accounts" || "$loaded_accounts" -lt "$BAD_ACCOUNTS" ]]; then
    echo "hot profile needs at least ${BAD_ACCOUNTS} accounts loaded, health has ${loaded_accounts:-none}" >&2
    echo "run: bash $(cd "$(dirname "$0")" && pwd)/reseed.sh ${BAD_ACCOUNTS}" >&2
    exit 1
  fi
fi

queries_of() {
  printf '%s' "$1" | sed -n 's/.*"character_queries":\([0-9][0-9]*\).*/\1/p'
}

run_one() {
  local name="$1"
  local url="$2"
  local mode="$3"
  local accounts="$4"
  echo
  echo "===== ${name} ${mode} accounts=${accounts} ====="
  local before="" after=""
  if [[ "$name" == "rust" && "$mode" == "hit" ]]; then
    before="$(curl -sf "$HEALTH_URL" || true)"
    echo "health_before ${before}"
  fi
  local health_arg=()
  if [[ "$name" == "rust" && "$mode" == "hit" ]]; then
    health_arg=(--health "$HEALTH_URL")
  fi
  taskset -c "$TASKSET_CPUS" "$BENCH" \
    --url "$url" \
    --mode "$mode" \
    --accounts "$accounts" \
    --warmup "$WARMUP" \
    --duration "$DURATION" \
    --threads "$THREADS" \
    --connections "$CONNECTIONS" \
    "${health_arg[@]}"
  if [[ "$name" == "rust" && "$mode" == "hit" ]]; then
    after="$(curl -sf "$HEALTH_URL" || true)"
    echo "health_after ${after}"
    local q0 q1
    q0="$(queries_of "$before")"
    q1="$(queries_of "$after")"
    if [[ -n "$q0" && -n "$q1" ]]; then
      echo "character_queries_including_warmup $((q1 - q0))"
    fi
  fi
}

TARGETS="${TARGETS:-rust php}"
for target in $TARGETS; do
  case "$target" in
    rust) url="$RUST_URL" ;;
    php) url="$PHP_URL" ;;
    *)
      echo "unknown target $target (use rust or php)" >&2
      exit 1
      ;;
  esac
  run_one "$target" "$url" hit "$HIT_ACCOUNTS"
  run_one "$target" "$url" bad "$BAD_ACCOUNTS"
  run_one "$target" "$url" miss "$MISS_ACCOUNTS"
done

echo
echo "done"
