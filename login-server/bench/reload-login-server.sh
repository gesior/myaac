#!/bin/bash
# Add updated_at tracking, rebuild login-server, and restart it.
# Run as root from the uploaded repo:
#   bash login-server/bench/reload-login-server.sh
set -euo pipefail

if [[ "$(id -u)" -ne 0 ]]; then
  echo "run as root" >&2
  exit 1
fi

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
DB_NAME="${DB_NAME:-otserv}"

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

echo "== updated_at =="
mysql --protocol=socket -uroot "$DB_NAME" < "$REPO/login-server/bench/accounts-updated-at.sql"

echo "== build =="
cargo build --release --manifest-path "$REPO/login-server/Cargo.toml"
install -m 755 "$REPO/login-server/target/release/login-server" /usr/local/bin/login-server

echo "== restart =="
systemctl restart login-server
for _ in $(seq 1 60); do
  if curl -sf http://127.0.0.1:8081/health >/dev/null; then
    break
  fi
  sleep 1
done
curl -sf http://127.0.0.1:8081/health
echo
journalctl -u login-server -n 20 --no-pager
echo
echo "look for incremental=true in the log line 'loaded account snapshot'"
