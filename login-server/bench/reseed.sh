#!/bin/bash
# Replace bench accounts. Default 50000 accounts, 3 characters each.
# login-server reloads the account list on its own; this waits until /health matches.
#   bash login-server/bench/reseed.sh
#   bash login-server/bench/reseed.sh 50000
set -euo pipefail

if [[ "$(id -u)" -ne 0 ]]; then
  echo "run as root" >&2
  exit 1
fi

COUNT="${1:-50000}"
DB_NAME="${DB_NAME:-otserv}"
HEALTH_URL="${HEALTH_URL:-http://127.0.0.1:8081/health}"
HERE="$(cd "$(dirname "$0")" && pwd)"

if [[ ! "$COUNT" =~ ^[0-9]+$ || "$COUNT" -lt 1 || "$COUNT" -gt 99999 ]]; then
  echo "account count must be 1..99999" >&2
  exit 1
fi

echo "seeding ${COUNT} accounts"
python3 "$HERE/seed.py" "$COUNT" | mysql --protocol=socket -uroot "$DB_NAME"

expected_players="$((COUNT * 3))"
read -r account_count player_count < <(mysql --protocol=socket -uroot -N -e "SELECT (SELECT COUNT(*) FROM accounts), (SELECT COUNT(*) FROM players);" "$DB_NAME")
if [[ "$account_count" != "$COUNT" || "$player_count" != "$expected_players" ]]; then
  echo "seed failed: accounts=${account_count} players=${player_count}" >&2
  exit 1
fi

echo "waiting for login-server to load ${COUNT} accounts"
ok=0
for _ in $(seq 1 90); do
  if curl -sf "$HEALTH_URL" | grep -Eq "\"accounts\":${COUNT}[,}]"; then
    ok=1
    break
  fi
  sleep 1
done
if [[ "$ok" -ne 1 ]]; then
  echo "login-server did not report ${COUNT} accounts" >&2
  curl -sf "$HEALTH_URL" || true
  echo >&2
  exit 1
fi

curl -sf "$HEALTH_URL"
echo
echo "seeded accounts=${account_count} players=${player_count}"
