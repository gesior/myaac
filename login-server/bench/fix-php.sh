#!/bin/bash
# Apply the PHP/nginx fix on a machine that already ran setup-vps.sh, rebuild
# login-bench, then check login.php with curl.
#   bash login-server/bench/fix-php.sh
set -euo pipefail

if [[ "$(id -u)" -ne 0 ]]; then
  echo "run as root" >&2
  exit 1
fi

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
SITE="/etc/nginx/sites-available/login-bench"

if [[ ! -f "$SITE" ]]; then
  echo "missing $SITE; run setup-vps.sh first" >&2
  exit 1
fi

if ! grep -q 'chunked_transfer_encoding off' "$SITE"; then
  sed -i 's/listen 127.0.0.1:8082;/listen 127.0.0.1:8082;\n\tchunked_transfer_encoding off;/' "$SITE"
fi
if ! grep -q 'fastcgi_buffer_size' "$SITE"; then
  sed -i 's/fastcgi_read_timeout 30s;/fastcgi_read_timeout 30s;\n\t\tfastcgi_buffer_size 32k;\n\t\tfastcgi_buffers 8 32k;/' "$SITE"
fi

nginx -t
systemctl reload nginx

if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi
cargo build --release --manifest-path "$REPO/login-server/bench/Cargo.toml"
install -m 755 "$REPO/login-server/bench/target/release/login-bench" /usr/local/bin/login-bench

bash "$REPO/login-server/bench/check-php.sh"
