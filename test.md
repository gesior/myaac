### RESULTS

Tested on 16 cores VPS with MySQL, PHP, nginx and Rust assigned to the first 4 cores.

- PHP `login.php` spends most of CPU on PHP and MySQL
- Rust `login.php` spends most of CPU on Rust

PHP `login.php` processes below 1000 requests per second and heavily loads MySQL (may lag OTS).
Rust `login.php` processes 39-52k requests per second and uses below 1% of single core on MySQL on cached requests. For missed cache requests, it sends requests to MySQL and uses around 100% of single core on MySQL.
The biggest benefit is that Rust `login.php` uses zero CPU on MySQL for not existing accounts or wrong passwords (they are loaded using a separate thread). It also caches 'players on account list' for 5 seconds, so it uses zero CPU on MySQL for spam of logins to the same account.

Normal test:
```
bash login-server/bench/run.sh


profile: spread
hit_accounts: 10000
bad_accounts: 10000
miss_accounts: 10000

===== rust hit accounts=10000 =====
health_before {"accounts":50000,"character_queries":800,"snapshot_age_ms":325623}
cpus_allowed: 4-15
url: http://127.0.0.1:8080/login.php
mode: hit
threads: 12
connections: 256
accounts: 10000
warmup_s: 10
measure_s: 30
requests: 1185169
matched: 1185169
mismatch: 0
io_errors: 0
rps: 39486.4
p50_ms: 0.247
p99_ms: 1.597
match_ratio: 1.0000
character_queries_measured: 60000
health_after {"accounts":50000,"character_queries":80800,"snapshot_age_ms":365679}
character_queries_including_warmup 80000

===== rust bad accounts=10000 =====
cpus_allowed: 4-15
url: http://127.0.0.1:8080/login.php
mode: bad
threads: 12
connections: 256
accounts: 10000
warmup_s: 10
measure_s: 30
requests: 1519764
matched: 1519764
mismatch: 0
io_errors: 0
rps: 50627.9
p50_ms: 0.225
p99_ms: 0.521
match_ratio: 1.0000

===== rust miss accounts=10000 =====
cpus_allowed: 4-15
url: http://127.0.0.1:8080/login.php
mode: miss
threads: 12
connections: 256
accounts: 10000
warmup_s: 10
measure_s: 30
requests: 1565674
matched: 1565674
mismatch: 0
io_errors: 0
rps: 52157.2
p50_ms: 0.216
p99_ms: 0.509
match_ratio: 1.0000

===== php hit accounts=10000 =====
cpus_allowed: 4-15
url: http://127.0.0.1:8082/login.php
mode: hit
threads: 12
connections: 256
accounts: 10000
warmup_s: 10
measure_s: 30
requests: 23631
matched: 23631
mismatch: 0
io_errors: 0
rps: 787.5
p50_ms: 15.281
p99_ms: 20.089
match_ratio: 1.0000

===== php bad accounts=10000 =====
cpus_allowed: 4-15
url: http://127.0.0.1:8082/login.php
mode: bad
threads: 12
connections: 256
accounts: 10000
warmup_s: 10
measure_s: 30
requests: 27190
matched: 27190
mismatch: 0
io_errors: 0
rps: 906.1
p50_ms: 13.268
p99_ms: 17.851
match_ratio: 1.0000

===== php miss accounts=10000 =====
cpus_allowed: 4-15
url: http://127.0.0.1:8082/login.php
mode: miss
threads: 12
connections: 256
accounts: 10000
warmup_s: 10
measure_s: 30
requests: 27412
matched: 27412
mismatch: 0
io_errors: 0
rps: 913.5
p50_ms: 13.181
p99_ms: 17.684
match_ratio: 1.0000

done
```

High cache hit test:
```
bash login-server/bench/reseed.sh 50000
PROFILE=hot bash login-server/bench/run.sh

seeding 50000 accounts
accounts
50000
players
150000
waiting for login-server to load 50000 accounts
{"accounts":50000,"character_queries":0,"snapshot_age_ms":1111}
seeded accounts=50000 players=150000
profile: hot
hit_accounts: 100
bad_accounts: 50000
miss_accounts: 50000

===== rust hit accounts=100 =====
health_before {"accounts":50000,"character_queries":0,"snapshot_age_ms":1148}
cpus_allowed: 4-15
url: http://127.0.0.1:8080/login.php
mode: hit
threads: 12
connections: 256
accounts: 100
warmup_s: 10
measure_s: 30
requests: 1463774
matched: 1463774
mismatch: 0
io_errors: 0
rps: 48763.1
p50_ms: 0.230
p99_ms: 0.517
match_ratio: 1.0000
character_queries_measured: 600
health_after {"accounts":50000,"character_queries":800,"snapshot_age_ms":41206}
character_queries_including_warmup 800

===== rust bad accounts=50000 =====
cpus_allowed: 4-15
url: http://127.0.0.1:8080/login.php
mode: bad
threads: 12
connections: 256
accounts: 50000
warmup_s: 10
measure_s: 30
requests: 1547886
matched: 1547886
mismatch: 0
io_errors: 0
rps: 51560.7
p50_ms: 0.220
p99_ms: 0.545
match_ratio: 1.0000

===== rust miss accounts=50000 =====
cpus_allowed: 4-15
url: http://127.0.0.1:8080/login.php
mode: miss
threads: 12
connections: 256
accounts: 50000
warmup_s: 10
measure_s: 30
requests: 1601576
matched: 1601576
mismatch: 0
io_errors: 0
rps: 53358.2
p50_ms: 0.210
p99_ms: 0.514
match_ratio: 1.0000

===== php hit accounts=100 =====
cpus_allowed: 4-15
url: http://127.0.0.1:8082/login.php
mode: hit
threads: 12
connections: 256
accounts: 100
warmup_s: 10
measure_s: 30
requests: 23676
matched: 23676
mismatch: 0
io_errors: 0
rps: 789.0
p50_ms: 15.252
p99_ms: 19.868
match_ratio: 1.0000

===== php bad accounts=50000 =====
cpus_allowed: 4-15
url: http://127.0.0.1:8082/login.php
mode: bad
threads: 12
connections: 256
accounts: 50000
warmup_s: 10
measure_s: 30
requests: 27207
matched: 27207
mismatch: 0
io_errors: 0
rps: 906.7
p50_ms: 13.266
p99_ms: 17.894
match_ratio: 1.0000

===== php miss accounts=50000 =====
cpus_allowed: 4-15
url: http://127.0.0.1:8082/login.php
mode: miss
threads: 12
connections: 256
accounts: 50000
warmup_s: 10
measure_s: 30
requests: 27400
matched: 27400
mismatch: 0
io_errors: 0
rps: 913.1
p50_ms: 13.174
p99_ms: 17.740
match_ratio: 1.0000

done

```
