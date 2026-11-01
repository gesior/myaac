#!/bin/sh
set -eu
BASE=http://login-server:8081

echo "waiting for login-server"
ok=0
i=0
while [ "$i" -lt 90 ]; do
  if wget -qO- -T 2 "$BASE/health" > /tmp/health 2>/dev/null; then
    ok=1
    break
  fi
  i=$((i + 1))
  sleep 1
done
if [ "$ok" -ne 1 ]; then
  echo "login-server did not become ready" >&2
  exit 1
fi
echo "health $(cat /tmp/health)"

queries() {
  wget -qO- -T 5 "$BASE/health" > /tmp/health
  sed -n 's/.*"character_queries":\([0-9][0-9]*\).*/\1/p' /tmp/health
}

expect_queries() {
  q=$(queries)
  if [ "$q" != "$1" ]; then
    echo "expected character_queries=$1 got ${q:-empty} health=$(cat /tmp/health)" >&2
    exit 1
  fi
}

post() {
  wget -qO- -T 5 --header="Content-Type: application/json" --post-data="$1" "$BASE/login.php" > /tmp/body
}

has() {
  if ! grep -q -F -- "$1" /tmp/body; then
    echo "missing [$1] in $(cat /tmp/body)" >&2
    exit 1
  fi
}

lacks() {
  if grep -q -F -- "$1" /tmp/body; then
    echo "unexpected [$1] in $(cat /tmp/body)" >&2
    exit 1
  fi
}

if ! grep -q '"accounts":3' /tmp/health; then
  echo "expected 3 accounts in $(cat /tmp/health)" >&2
  exit 1
fi

post '{"type":"login","accountname":"missing","password":"nope"}'
has "Account name or password is not correct."
has '"errorCode":3'
expect_queries 0

post '{"type":"login","accountname":"tester","password":"wrong"}'
has "Account name or password is not correct."
expect_queries 0

post '{"type":"login","accountname":"otpuser","password":"secret"}'
has "Submit a valid two-factor authentication token."
has '"errorCode":6'
expect_queries 0

post '{"type":"login","accountname":"otpuser","password":"secret","token":"abcdef"}'
has "Two-factor authentication failed, token is wrong."
expect_queries 0

post '{"type":"login","email":"tester@example.com","password":"secret"}'
has '"ispremium":false'
has '"premiumuntil":0'
has '"characters":[]'
has 'tester@example.com\nsecret\n\n'
expect_queries 1

post '{"type":"login","email":"tester@example.com","password":"secret"}'
has '"characters":[]'
expect_queries 1

post '{"type":"login","email":"hero@example.com","password":"secret"}'
has '"ispremium":true'
has '"name":"High"'
has '"name":"Low"'
has '"vocation":"Elite Knight"'
has '"vocation":"Sorcerer"'
has '"ismaincharacter":true'
has '"ismaincharacter":false'
has '"ismale":true'
has '"ismale":false'
has '"level":50'
has "TestWorld"
has '"externalport":7172'
has 'hero@example.com\nsecret\n\n'
lacks '"name":"Ghost"'
expect_queries 2

post '{"type":"login","accountname":"hero","password":"secret"}'
has 'hero\nsecret\n\n'
has '"name":"High"'
expect_queries 2

post '{"type":"cacheinfo"}'
has "Unrecognized event cacheinfo."

post '{'
has "Invalid request."

echo "ok"
