#!/bin/bash
# Install the 4-core login benchmark on Ubuntu 24.04.
# Upload this repo, then as root:
#   bash login-server/bench/setup-vps.sh
#   bash login-server/bench/run.sh
set -euo pipefail

if [[ "$(id -u)" -ne 0 ]]; then
  echo "run as root" >&2
  exit 1
fi

if [[ "$(nproc)" -lt 16 ]]; then
  echo "need at least 16 CPUs so services can use 0-3 and the bench 4-15 (found $(nproc))" >&2
  exit 1
fi

export DEBIAN_FRONTEND=noninteractive
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
DB_NAME="otserv"
DB_USER="otserv"
DB_PASS="benchdbpass"
SEED_ACCOUNTS="${SEED_ACCOUNTS:-10000}"
OT_DIR="/var/otserv-bench"
SESSIONS="${REPO}/system/php_sessions"

if [[ ! -f "$REPO/login.php" || ! -f "$REPO/login-server/Cargo.toml" ]]; then
  echo "repo root looks wrong: $REPO" >&2
  exit 1
fi

echo "== packages =="
apt-get update
apt-get install -y \
  nginx \
  php8.3-fpm \
  php8.3-cli \
  php8.3-mysql \
  php8.3-xml \
  php8.3-mbstring \
  php8.3-curl \
  php8.3-zip \
  php8.3-gd \
  php8.3-apcu \
  php8.3-opcache \
  composer \
  mariadb-server \
  git \
  curl \
  ca-certificates \
  build-essential \
  pkg-config \
  libssl-dev \
  python3 \
  unzip

echo "== rust =="
if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
fi
if [[ -f "$HOME/.cargo/env" ]]; then
  # shellcheck disable=SC1091
  source "$HOME/.cargo/env"
fi

echo "== mariadb =="
cat > /etc/mysql/mariadb.conf.d/99-bench.cnf <<'EOF'
[mysqld]
bind-address = 127.0.0.1
innodb_buffer_pool_size = 1G
max_connections = 300
skip-log-bin
innodb_flush_log_at_trx_commit = 2
EOF

DB_UNIT="mariadb"
if ! systemctl list-unit-files --type=service --no-legend | awk '{print $1}' | grep -qx 'mariadb.service'; then
  DB_UNIT="mysql"
fi
systemctl enable --now "$DB_UNIT"
systemctl restart "$DB_UNIT"
for _ in $(seq 1 60); do
  if mysqladmin --protocol=socket -uroot ping --silent; then
    break
  fi
  sleep 1
done
mysqladmin --protocol=socket -uroot ping --silent

mysql --protocol=socket -uroot <<EOF
DROP DATABASE IF EXISTS \`${DB_NAME}\`;
CREATE DATABASE \`${DB_NAME}\` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
DROP USER IF EXISTS '${DB_USER}'@'127.0.0.1';
CREATE USER '${DB_USER}'@'127.0.0.1' IDENTIFIED BY '${DB_PASS}';
GRANT ALL PRIVILEGES ON \`${DB_NAME}\`.* TO '${DB_USER}'@'127.0.0.1';
FLUSH PRIVILEGES;
EOF

echo "== schema =="
curl -fsSL -o /tmp/gesior-schema.sql \
  https://raw.githubusercontent.com/gesior/forgottenserver-gesior/compilation/schema.sql
sed 's/\r$//' /tmp/gesior-schema.sql | mysql --protocol=socket -uroot "$DB_NAME"
sed 's/\r$//' "$REPO/login-server/bench/accounts-updated-at.sql" | mysql --protocol=socket -uroot "$DB_NAME"
sed 's/\r$//' "$REPO/install/includes/schema.sql" | mysql --protocol=socket -uroot "$DB_NAME"
python3 "$REPO/login-server/bench/seed.py" "$SEED_ACCOUNTS" | mysql --protocol=socket -uroot "$DB_NAME"
mysql --protocol=socket -uroot "$DB_NAME" <<'EOF'
INSERT INTO `myaac_config` (`name`, `value`) VALUES ('database_version', '46')
  ON DUPLICATE KEY UPDATE `value` = VALUES(`value`);
INSERT INTO `myaac_settings` (`name`, `key`, `value`)
  VALUES ('core', 'account_login_ipban_protection', 'false');
EOF
expected_players="$((SEED_ACCOUNTS * 3))"
read -r account_count player_count < <(mysql --protocol=socket -uroot -N -e "SELECT (SELECT COUNT(*) FROM accounts), (SELECT COUNT(*) FROM players);" "$DB_NAME")
if [[ "$account_count" != "$SEED_ACCOUNTS" || "$player_count" != "$expected_players" ]]; then
  echo "seed failed: accounts=${account_count} players=${player_count}" >&2
  exit 1
fi
echo "seeded accounts=${account_count} players=${player_count}"

echo "== ots config.lua =="
mkdir -p "$OT_DIR"
cat > "$OT_DIR/config.lua" <<EOF
serverName = "Bench"
ip = "127.0.0.1"
gameProtocolPort = 7172
worldType = "pvp"
mysqlHost = "127.0.0.1"
mysqlPort = 3306
mysqlUser = "${DB_USER}"
mysqlPass = "${DB_PASS}"
mysqlDatabase = "${DB_NAME}"
EOF

echo "== myaac =="
cat > "$REPO/config.local.php" <<EOF
<?php
\$config['installed'] = true;
\$config['env'] = 'prod';
\$config['server_path'] = '${OT_DIR}/';
\$config['cache_engine'] = 'apcu';
\$config['gzip_output'] = false;
\$config['database_auto_migrate'] = false;
EOF

export COMPOSER_ALLOW_SUPERUSER=1
composer install --no-dev --no-interaction --prefer-dist --working-dir "$REPO"

mkdir -p "$REPO/system/cache/twig" "$REPO/system/logs" "$SESSIONS"
chown -R www-data:www-data "$REPO/system/cache" "$REPO/system/logs"
chmod -R a+rX "$REPO"
dir="$REPO"
while [[ "$dir" != "/" ]]; do
  chmod o+x "$dir"
  dir="$(dirname "$dir")"
done
www_uid="$(id -u www-data)"
www_gid="$(id -g www-data)"
if ! mountpoint -q "$SESSIONS"; then
  mount -t tmpfs -o "size=1G,mode=0750,uid=${www_uid},gid=${www_gid}" tmpfs "$SESSIONS"
fi
if ! grep -qF "$SESSIONS" /etc/fstab; then
  echo "tmpfs ${SESSIONS} tmpfs size=1G,mode=0750,uid=${www_uid},gid=${www_gid},noatime 0 0" >> /etc/fstab
fi
if [[ "$(readlink -f "$REPO")" != "$(readlink -f /var/www/myaac 2>/dev/null || true)" ]]; then
  if [[ -e /var/www/myaac && ! -L /var/www/myaac ]]; then
    echo "/var/www/myaac exists and is not a symlink" >&2
    exit 1
  fi
  ln -sfn "$REPO" /var/www/myaac
fi

PHP_POOL="/etc/php/8.3/fpm/pool.d/www.conf"
sed -i 's/^pm = .*/pm = static/' "$PHP_POOL"
sed -i 's/^pm.max_children = .*/pm.max_children = 16/' "$PHP_POOL"
if grep -q '^pm.max_requests' "$PHP_POOL"; then
  sed -i 's/^pm.max_requests = .*/pm.max_requests = 0/' "$PHP_POOL"
else
  sed -i 's/^;pm.max_requests = .*/pm.max_requests = 0/' "$PHP_POOL"
fi
cat > /etc/php/8.3/fpm/conf.d/99-bench.ini <<'EOF'
opcache.enable=1
opcache.enable_cli=0
opcache.memory_consumption=128
opcache.interned_strings_buffer=16
opcache.max_accelerated_files=20000
opcache.validate_timestamps=0
apc.enable_cli=0
EOF

echo "== login-server =="
cat > /etc/login-server.toml <<EOF
listen = "127.0.0.1:8081"

[database]
host = "127.0.0.1"
port = 3306
user = "${DB_USER}"
password = "${DB_PASS}"
name = "${DB_NAME}"
pool_size = 16

[cache]
account_refresh_secs = 1
characters_ttl_secs = 5

[password]
algorithm = "sha1"
database_salt = ""

[world]
id = 0
name = "Bench"
ip = "127.0.0.1"
port = 7172
location = "BRA"
pvp_type = "pvp"

[security]
email_verify = false
ip_ban = false
max_attempts = 5
ban_minutes = 30
trust_proxy = true

[premium]
lastday_is_premium_end = false

[vocations]
amount = 4
names = ["None", "Sorcerer", "Druid", "Paladin", "Knight", "Master Sorcerer", "Elder Druid", "Royal Paladin", "Elite Knight"]
EOF

cargo build --release --manifest-path "$REPO/login-server/Cargo.toml"
cargo build --release --manifest-path "$REPO/login-server/bench/Cargo.toml"
install -m 755 "$REPO/login-server/target/release/login-server" /usr/local/bin/login-server
install -m 755 "$REPO/login-server/bench/target/release/login-bench" /usr/local/bin/login-bench

cat > /etc/systemd/system/login-server.service <<'EOF'
[Unit]
Description=AAC login server
After=network.target mariadb.service mysql.service

[Service]
Type=simple
ExecStart=/usr/local/bin/login-server /etc/login-server.toml
Restart=on-failure
RestartSec=1
LimitNOFILE=1048576

[Install]
WantedBy=multi-user.target
EOF

echo "== nginx =="
sed -i 's/^worker_processes .*/worker_processes 4;/' /etc/nginx/nginx.conf
sed -i 's/worker_connections .*/worker_connections 4096;/' /etc/nginx/nginx.conf
cat > /etc/nginx/sites-available/login-bench <<'EOF'
upstream login_server {
	server 127.0.0.1:8081;
	keepalive 256;
}

server {
	listen 127.0.0.1:8080;
	access_log off;
	error_log /var/log/nginx/login-bench-error.log warn;
	keepalive_requests 1000000;
	gzip off;
	client_max_body_size 8k;

	location = /login.php {
		proxy_http_version 1.1;
		proxy_set_header Connection "";
		proxy_set_header Host $host;
		proxy_set_header X-Real-IP $remote_addr;
		proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
		proxy_pass http://login_server;
		proxy_read_timeout 30s;
	}

	location = /health {
		proxy_http_version 1.1;
		proxy_set_header Connection "";
		proxy_set_header Host $host;
		proxy_pass http://login_server;
	}
}

server {
	listen 127.0.0.1:8082;
	root /var/www/myaac;
	access_log off;
	error_log /var/log/nginx/login-bench-php-error.log warn;
	keepalive_requests 1000000;
	gzip off;
	chunked_transfer_encoding off;
	client_max_body_size 8k;

	location = /login.php {
		include snippets/fastcgi-php.conf;
		fastcgi_pass unix:/run/php/php8.3-fpm.sock;
		fastcgi_read_timeout 30s;
		fastcgi_buffer_size 32k;
		fastcgi_buffers 8 32k;
		fastcgi_keep_conn on;
	}
}
EOF
ln -sfn /etc/nginx/sites-available/login-bench /etc/nginx/sites-enabled/login-bench
nginx -t

echo "== cpu affinity 0-3 =="
for unit in nginx php8.3-fpm "$DB_UNIT" login-server; do
  mkdir -p "/etc/systemd/system/${unit}.service.d"
  cat > "/etc/systemd/system/${unit}.service.d/affinity.conf" <<'EOF'
[Service]
CPUAffinity=0-3
LimitNOFILE=1048576
EOF
done

systemctl daemon-reload
systemctl enable login-server
systemctl restart php8.3-fpm
systemctl restart nginx
systemctl restart "$DB_UNIT"
systemctl restart login-server

echo "== warmup =="
health_ok=0
for _ in $(seq 1 60); do
  if curl -sf http://127.0.0.1:8081/health | grep -Eq "\"accounts\":${SEED_ACCOUNTS}[,}]"; then
    health_ok=1
    break
  fi
  sleep 1
done
if [[ "$health_ok" -ne 1 ]]; then
  echo "login-server did not load ${SEED_ACCOUNTS} accounts" >&2
  journalctl -u login-server -n 80 --no-pager >&2 || true
  exit 1
fi
curl -sf http://127.0.0.1:8081/health
echo
rust_body="$(curl -sf -m 30 -X POST http://127.0.0.1:8080/login.php \
  -H 'Content-Type: application/json' \
  -d '{"type":"login","accountname":"bench00001","password":"secret"}')"
printf '%s\n' "$rust_body" | grep -q sessionkey
php_body="$(curl -sf -m 120 -X POST http://127.0.0.1:8082/login.php \
  -H 'Content-Type: application/json' \
  -d '{"type":"login","accountname":"bench00001","password":"secret"}')"
if ! printf '%s\n' "$php_body" | grep -q sessionkey; then
  echo "PHP login did not return sessionkey:" >&2
  printf '%s\n' "$php_body" >&2
  exit 1
fi

echo "== affinity =="
for unit in nginx php8.3-fpm "$DB_UNIT" login-server; do
  pid="$(systemctl show -p MainPID --value "$unit")"
  cpus="$(awk '/Cpus_allowed_list/ {print $2}' "/proc/${pid}/status")"
  echo "${unit} pid=${pid} cpus=${cpus}"
done

echo
echo "ready. Run: bash ${REPO}/login-server/bench/run.sh"
