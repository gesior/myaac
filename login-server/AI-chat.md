# Login server implementation
_Exported on 11/1/2026 at 09:45:02 GMT+1 from Cursor (3.27.22)_

---

**User**

@login.php mam tutaj skrypt na logowanie do konta w grze, jednak obsluga logowania w ten sposob jest niewydajna i bardzo latwa do przeciazenia dowolny atakiem DDoS
chce przepisac ta podstrone jako oddzielna aplikacje w Rust, Go, C++ lub innym wydajnym wielowatkowym jezyku ze wspoldzielona pamiecia miedzy watkami (pod wydajny cache)
teraz kazde zapytanie do login.php laduje dane z bazy danych
nowa aplikacja ma co sekunde (do konfiguracji) aktualizowac spis kont z bazy danych w oddzielnym wątku z workerem do aktualizacji danych o kontach - aktualizacja danych o kontach ma minimalnie blokowac trwajaca obsluge logowania w innych watkach, najlepiej jak by cala struktura danych (cache) byla podmieniana, a watki uzywajace starej struktury uzywaly jej do czasu zakonczenia obslugi swoich zapytan o logowanie (jakis garbage collector?)
jesli konto istnieje - jest w spisie kont zaladowanych z bazy - to nalezy zaladowac spis postaci, ale spis postaci z konta ma byc trzymane w cache przez 5 sekund (do konfiguracji), tak, zeby masowe logowani na 1 konto nie wywolywalo wielu zapytan SQL

celem jest uzyskanie serwera, ktory bedzie przyjmowal zapytania HTTP z JSON i odpowiadal na nie dziesiatki-setki tysiecy razy na sekunde

obsluga SSL i przekierowaniem zapytan do nowej aplikacji zajmie sie nginx, przygotuj konfig, zeby przekierowac zapytania do `/login.php` do nowego serwera od obslugi logowania

na tym komputerze nie ma C++, Rust, ani Go, do testow uzywaj dockera - utworz Dockerfile

nowa aplikacje utworz w katalogu 'login-server'

---

**Cursor**

Najpierw przejrzę `login.php` i sposób, w jaki logowanie korzysta z bazy, żeby zaplanować serwer z cache’em kont i postaci.

To jest protokół logowania klienta gry. Sprawdzę hashowanie haseł, schemat kont i postaci oraz limity, bo od tego zależy kształt cache.

---

**User**

nowy serwer ma tylko obslugiwac zapytania do `case 'login':`, inne dane (z XML itp.) dalej beda ladowane z PHP, tylko 'login' nie moze padac podczas ddos, dlatego bedzie na oddzielnym serwerze
logowanie bedzie szlo przez `login.php` w nginx, a strona `login.php` zmieni potem adres na `info.php` i w kliencie zmienia ustawienia skad co ladowac

---

**Cursor**

Serwer ma obsługiwać wyłącznie `login`; reszta zostaje w PHP. Sprawdzam jeszcze premium i schemat haseł, potem złożę plan podmiany cache bez blokowania logowania.

---

**User**

Implement the plan as specified, it is attached for your reference. Do NOT edit the plan file itself.

To-do's from the plan have already been created. Do not create them again. Mark them as in_progress as you work, starting with the first one. Don't stop until you have completed all the to-dos.

---

**Cursor**

Serwer ma obsługiwać tylko `type=login`. Zaczynam od szkieletu w Rust: konfiguracja, protokół JSON i podmiana cache kont.

Piszę serwer jako osobny katalog: konfiguracja, odczyt bazy i obsługa samego `login`.

Line endings are LF. Next I’ll build the image in Docker and run the login tests.

Serwer logowania jest w katalogu `login-server`. Przyjmuje tylko JSON z `type=login` i odpowiada tak jak gałąź `case 'login'` w `login.php`. `cacheinfo`, eventy i boosted creature zostają w PHP.

Spis kont jest w pamięci. Osobny wątek co sekundę (do ustawienia) buduje nową mapę i podmienia ją przez `ArcSwap`. Wątek logowania trzyma swoją kopię wskaźnika do końca sprawdzenia konta, więc podmiana go nie blokuje. Stara mapa znika, gdy ostatni request ją puści.

Postacie są dociągane z MySQL dopiero po trafieniu konta i zgodnym haśle (oraz 2FA, jeśli jest). Wynik siedzi w cache przez 5 sekund. Równoległe logowania na to samo konto przy zimnym cache dzielą jedno zapytanie SQL.

Nginx przekazuje `POST /login.php` do `127.0.0.1:8081` (`proxy_pass`, nie przekierowanie HTTP). Wpis jest w `nginx-sample.conf` i w `login-server/nginx.conf`. Blok `upstream` musi być w kontekście `http`, a `location = /login.php` przed obsługą PHP. Po włączeniu tej lokacji PHP pod `/login.php` przestaje być widoczne — do tego momentu klient musi brać resztę danych z `info.php`.

Konfiguracja: `login-server/config.example.toml` (baza, świat, hasło, premium, profesje, limity). Start: `login-server /ścieżka/config.toml` albo zmienna `LOGIN_CONFIG`.

W Dockerze test logowania przeszedł (email i nazwa konta, złe hasło, nieznane konto, 2FA, jedno zapytanie o postacie na konto w oknie cache). Ścieżka nieznanego konta, czyli typowy DDoS bez SQL, zrobiła około **513 tysięcy zapytań na sekundę** w Docker Desktop, wszystkie z kodem 200.

```text
docker compose up --build --abort-on-container-exit --exit-code-from test
docker compose --profile bench up --abort-on-container-exit --exit-code-from bench login-server bench
```

---

**User**

chce przeprowadzic testy wydajnosci, w tym celu potrzebuje jakis kod na benchmark
testy bede prowadzil na VPS z 16 rdzeniowym CPU, jako serwera www chce uzyc nginx, a zamiast mysql uzyc mariadb
chce, zeby mysql, nginx, PHP i serwer rust dzialaly na wybranych 4 rdzeniach CPU - tak zeby porownac jak cala konfiguracja zachowuje sie na VPS z 4 rdzeniami (moze np. cpu affinity)
pozostale 12 rdzeni zostawiam dla procesu benchmarka - ten ma wybrac do uzycia pozostale rdzenie
na serwerze bedzie system Ubuntu 24.04, a wszystkie komendy beda odpalane z uzytkownika root (to serwer w cloud)
przygotuj skrypt (spis komend SSH), ktory zainstaluje i skonfiguruje wszystko pod takie testy
jako bazy danych uzyj https://github.com/gesior/forgottenserver-gesior/blob/compilation/schema.sql
w bazie wstaw 10k kont i po 3 postacie na konto, test ma sprawdzic wydajnosc zapytan na sekunde do istniejacych kont, logowan ze zlym haslem i logowan do nie istniejacych kont

---

**Cursor**

Żeby skrypt seedował właściwe tabele i benchmark trafiał w ten sam protokół co klient, sprawdzę schemat Gesiora oraz to, jak serwer Rusta i nginx przyjmują logowanie.

Schemat Gesiora ma `premium_ends_at` i `players.deletion`, a hasła są SHA1 — serwer Rusta to już wykrywa. Plan obejmuje instalację na 4 rdzeniach i benchmark trzech ścieżek logowania.

---

**User**

Implement the plan as specified, it is attached for your reference. Do NOT edit the plan file itself.

To-do's from the plan have already been created. Do not create them again. Mark them as in_progress as you work, starting with the first one. Don't stop until you have completed all the to-dos.

---

**Cursor**

Zaczynam od programu benchmarku, potem skrypt instalacji VPS i uruchomienie sześciu pomiarów.

Schemat i PHP są jasne: wyłączę limit IP, a benchmark będzie sam podmieniał nazwy kont. Dopisuję też log liczby wątków Tokio, żeby na VPS było widać, że zostały 4.

Benchmark jest w `login-server/bench`. Na VPS z Ubuntu 24.04 wgrywasz to repozytorium i jako root odpalasz:

```bash
bash login-server/bench/setup-vps.sh
bash login-server/bench/run.sh
```

Instalacja stawia MariaDB, nginx, PHP 8.3 i serwer Rust. Baza to schemat Gesiora plus tabele MyAAC: 10 000 kont `bench00001`–`bench10000` (hasło `secret`, SHA1) i 3 postacie na konto. Nginx, PHP-FPM, MariaDB i `login-server` dostają maskę CPU `0-3`. Nginx ma 4 workery, a log serwera Rust pokazuje liczbę wątków Tokio po tej masce. Na końcu skrypt wypisuje `Cpus_allowed_list` każdej usługi.

Pomiar idzie przez nginx, po kolei, narzędziem `login-bench` przypiętym do rdzeni `4-15`:

| Cel | Adres |
|---|---|
| Rust | `http://127.0.0.1:8080/login.php` |
| PHP | `http://127.0.0.1:8082/login.php` |

Dla każdego są trzy tryby po 10 s rozgrzewki i 30 s pomiaru: poprawne hasło (`hit`), złe hasło (`bad`) i nieistniejące konto (`miss`). Konta lecą po kolei, nie w jedno. Program drukuje RPS, p50, p99 i odsetek odpowiedzi zgodnych z trybem. Przy `hit` na Rust dodatkowo podaje, ile zapytań o postacie weszło w samo okno pomiaru (`character_queries_measured`). Limit błędnych haseł po IP jest wyłączony, żeby piąta próba nie zmieniała odpowiedzi.

---

**User**

testy przerwaly sie po:
===== rust miss =====
cpus_allowed: 4-15
url: http://127.0.0.1:8080/login.php
mode: miss
threads: 12
connections: 256
accounts: 10000
warmup_s: 10
measure_s: 30
requests: 1565214
matched: 1565214
mismatch: 0
io_errors: 0
rps: 52141.0
p50_ms: 0.215
p99_ms: 0.517
match_ratio: 1.0000

===== php hit =====
cpus_allowed: 4-15
url: http://127.0.0.1:8082/login.php
mode: hit
threads: 12
connections: 256
accounts: 10000
warmup_s: 10
measure_s: 30
requests: 0
matched: 0
mismatch: 0
io_errors: 23646
rps: 0.0
p50_ms: 0.000
p99_ms: 0.000


moze trzeba zmienic cos w konfiguracji php/nginx, zeby testy dzialaly
mozesz napisac jakis skrypt curl, ktory sprawdzi czy logowanie przez PHP w benchmarku dziala poprawnie?

---

**Cursor**

PHP przy nginx często odsyła odpowiedź bez `Content-Length` (chunked), a benchmark takiego ciała nie umie odczytać — curl przy tym przechodzi. Sprawdzę parser i konfigurację, potem dodam skrypt kontrolny.

Rust dostaje `Content-Length`, a PHP za nginx często wysyła ciało jako `chunked`. Benchmark traktował to jak zerwane połączenie, stąd same błędy I/O przy zerze udanych requestów. Poprawiam odczyt odpowiedzi i dodaję skrypt `curl`, który sprawdzi logowanie PHP.

Test PHP nie doszedł do logowania. Nginx oddaje odpowiedź PHP jako `Transfer-Encoding: chunked`, bez `Content-Length`. Serwer Rust zawsze wysyła długość, więc jego testy przeszły. Benchmark liczył każdą odpowiedź PHP jako urwany odczyt: stąd `requests: 0` i kilkadziesiąt tysięcy `io_errors`.

Na VPS, po wgraniu tej wersji repozytorium, jako root:

```bash
bash login-server/bench/fix-php.sh
```

Skrypt wyłącza chunked encoding na porcie `8082`, przeładowuje nginx, buduje `login-bench` od nowa (umie chunked i odpowiedź zamykaną połączeniem) i od razu sprawdza logowanie curlem.

Sam curl, bez zmian w nginx:

```bash
bash login-server/bench/check-php.sh
```

Wysyła trzy żądania na `http://127.0.0.1:8082/login.php`: poprawne hasło `bench00001` / `secret`, złe hasło i nieistniejące konto. Przy sukcesie kończy się linią `ok: PHP login hit, bad password and missing account`. Nagłówki są wypisane, więc widać, czy odpowiedź jest chunked.

Pomiar samego PHP, bez powtórki Rusta:

```bash
TARGETS=php bash login-server/bench/run.sh
```

---

**User**

chcialbym zrobic drugi profil testowy, kiedy zapytania 'hit' czesciej trafiaja w cache, czyli zeby test odwolywal sie w kolko do tych samych 100 kont z 50k kont utworzonych

---

**Cursor**

Drugi profil ma trzymać w pamięci 50 tysięcy kont, a udane logowania kręcić tylko po stu z nich, żeby lista postaci zostawała w cache. Sprawdzę, jak teraz seed i benchmark wybierają konta.

Profil `spread` zostaje: logowanie idzie po wszystkich kontach. Profil `hot` najpierw wkłada 50 tysięcy kont, a udane logowania kręci tylko po pierwszych stu, więc lista postaci zostaje w cache.

Drugi profil nazywa się `hot`. W bazie jest 50 tysięcy kont, a udane logowanie kręci się tylko po `bench00001`–`bench00100`, więc lista postaci tych stu kont zostaje w cache. Złe hasło i nieistniejące konto nadal idą po całych 50 tysiącach.

Na VPS, jako root, po wgraniu tej wersji:

```bash
bash login-server/bench/reseed.sh 50000
PROFILE=hot bash login-server/bench/run.sh
```

`reseed.sh` kasuje dotychczasowe konta testowe, wstawia 50 tysięcy kont i 150 tysięcy postaci, potem czeka, aż `login-server` pokaże te 50 tysięcy w `/health`. Sam serwer nie wymaga restartu: spis kont i tak odświeża się co sekundę.

Dotychczasowy pomiar bez `PROFILE` dalej chodzi po 10 tysiącach kont, o ile baza nie została przesiana. Po `reseed` szeroki test to `ACCOUNTS=50000 bash login-server/bench/run.sh`.

---

**User**

czy login-server w rust pobiera w kolko wszystkie konta z bazy danych czy tylko te zmodyfikowane?
w innym serwerze logowania w Rust widzialem, ze serwer dodawal kolumne `updated_at` z trigerem i indeksem, zeby zmiany hasla itp. byly sledzone przez MySQL, a po pierwszym zaladowaniu wszystkich kont, mozna bylo tylko pobierac nowe i zmodyfikowane (trzymal w tym celu 2 listy: ostatnio przekazana i aktualnie budowana, zeby przekazanie listy kont blokowalo watki z 'logowaniem' na minimalny czas, a samo kopiowanie listy bylo robione w watku ladujacym zaktualizowane informacje z bazy danych)

przygotuj takie zmiany i komendy do SSH, zeby ponownie skompilowac serwer po zmianach i go zrestartowac na VPS

---

**Cursor**

Teraz serwer co sekundę czyta całą tabelę `accounts`. Zmienię to tak, żeby po pierwszym pełnym wczytaniu dociągał tylko nowe i zmienione wiersze, a podmiana spisu dalej była samą zamianą wskaźnika.

Pełne czytanie tabeli zostaje tylko na starcie. Potem wątek odświeżania dociąga wiersze z `updated_at` nowszym niż ostatni znacznik, składa nową listę u siebie i podmienia ją jednym wskaźnikiem.

Do tej pory login-server co sekundę czytał całą tabelę `accounts`. Teraz pełny odczyt jest tylko przy starcie. Potem wątek odświeżania bierze z MySQL wyłącznie wiersze z `updated_at` nowszym niż znacznik z poprzedniego cyklu, kopiuje opublikowaną listę u siebie i podmienia ją jednym wskaźnikiem. Wątki logowania trzymają starą listę do końca swojego żądania.

Śledzenie zmian robi MariaDB. Kolumna `accounts.updated_at` ma indeks, a triggery ustawiają ją przy `INSERT` i `UPDATE` (zmiana hasła, e-mail, premium). Usunięte konto zapisuje trigger do `accounts_removed`, więc znika ze spisu bez skanowania całej tabeli. Gdy w danym cyklu nic się nie zmieniło, lista w ogóle nie jest kopiowana.

Na VPS, po wgraniu tej wersji, jako root:

```bash
bash login-server/bench/reload-login-server.sh
```

To samo ręcznie:

```bash
mysql --protocol=socket -uroot otserv < login-server/bench/accounts-updated-at.sql
source "$HOME/.cargo/env"
cargo build --release --manifest-path login-server/Cargo.toml
install -m 755 login-server/target/release/login-server /usr/local/bin/login-server
systemctl restart login-server
journalctl -u login-server -n 20 --no-pager
```

W logu startu ma być `incremental=true`. Późniejsza zmiana hasła w bazie pojawia się w logu jako `applied account delta` z `changed=1`, a nie jako ponowne wczytanie wszystkich kont.

