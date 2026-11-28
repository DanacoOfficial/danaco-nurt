<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/supervisor.md -->
<!-- Responsibility:   Documents Job supervisor protocol. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Protokół nadzorcy zadań {#job-supervisor-protocol}

Uruchomienie, które trwa w tle po zakończeniu wiersza poleceń, jest nadzorowane
przez proces, który przeżywa wiersz poleceń. Wiersz poleceń i nadzorca wymieniają
pliki w katalogu stanu; nie ma gniazda ani innego kanału. Implementacja w Rust
znajduje się w `crates/nurt-proc/src/jobs`. Wersja protokołu to `1` (`PROTOCOL`
w `jobs/table.rs`); każdy plik komunikatu zawiera ją w polu `protocol`, a plik
innej wersji jest odrzucany.

## 1. Pliki `<state>/jobs` {#1-files-of-statejobs}

| Plik | Zawartość |
|---|---|
| `sequence`, `sequence.lock` | numer ostatniego zadania; identyfikatory `j<number>` nigdy nie są używane ponownie |
| `<id>.json` | plik zadania: rekord publiczny, żądanie bez środowiska, wynik |
| `<id>.input.json` | środowisko i standardowe wejście; do odczytu wyłącznie dla konta silnika |
| `<id>.lock` | blokowany przez proces nadzorujący zadanie; zadanie zapisane jako działające, którego blokada jest wolna, utraciło nadzorcę |
| `<id>.control/` | żądania sterujące kierowane do procesu nadzorującego i odpowiedzi na nie |
| `host.lock` | blokowany przez odłączonego nadzorcę |
| `inbox/` | żądania uruchomienia, odpowiedzi na nie i powiadomienia o anulowaniu |

Pliki są zastępowane atomowo przez zmianę nazwy kompletnego pliku tymczasowego,
dzięki czemu czytelnik nigdy nie widzi pliku częściowego i nie zakłada blokady.

## 2. Początek uruchomienia {#2-start-of-a-run}

1. Klient zapisuje `inbox/<name>.request.json`, gdzie `<name>` to
   `<process id>-<counter>`; żądania są obsługiwane w kolejności liczników.
   Plik zawiera `protocol`, `command_name`, powiązany `program`, `request`
   (uruchomienie wraz ze środowiskiem i standardowym wejściem) oraz `wait`, czas,
   przez jaki uruchomienie może trwać, zanim zostanie przeniesione do tła.
2. Gdy `host.lock` jest wolny, klient uruchamia nadzorcę poleceniem
   zarejestrowanym przez aplikację hosta i uruchamia go ponownie, jeśli blokada
   jest nadal wolna po 3 s. Uruchomiony nadzorca próbuje założyć blokadę do
   10 razy w odstępach 10 ms, aby chwila, w której klient sprawdza blokadę, nie
   spowodowała jego rezygnacji.
3. Nadzorca przejmuje żądanie, usuwając jego plik. Kto usunie plik, ten jest
   właścicielem żądania: klient, który usunie go jako pierwszy, wycofał żądanie,
   a nadzorca go nie uruchamia.
4. Nadzorca wykonuje przygotowanie uruchomienia (transakcja dziennika, wykrywanie
   zmian, bufor), uruchamia program, zapisuje proces w pliku zadania i zapisuje
   `inbox/<name>.reply.json`: `started` z identyfikatorem zadania albo `failed`
   z błędem. Klient czeka na odpowiedź do 30 s, usuwa ją i śledzi zadanie.

## 3. Anulowanie {#3-cancellation}

Anulowanie jest stanem wywołania, a nie zdarzeniem, które można pominąć. Jest
dostarczane na trzy sposoby, zależnie od tego, jak daleko uruchomienie
zaawansowało:

| Moment | Jak anulowanie dociera do uruchomienia |
|---|---|
| Nadzorca nie przejął żądania | klient usuwa plik żądania; wywołanie kończy się błędem `E_CANCELLED` i nic nie zostało uruchomione |
| Nadzorca przejął żądanie, a program się nie uruchomił | klient zapisuje `inbox/<name>.cancel.json` (`{"protocol": 1}`); nadzorca nadaje każdemu uruchomieniu własny token anulowania i anuluje go, gdy plik się pojawi; przygotowanie kończy się przy najbliższej kontroli, nadzorca sprawdza token ponownie bezpośrednio przed uruchomieniem programu, a żądanie otrzymuje odpowiedź `failed` (`E_CANCELLED`); wpis dziennika jest wycofywany z przyczyną `cancelled` i żadne zadanie nie jest zapisywane |
| Program się uruchomił | klient zapisuje żądanie sterujące `cancel` do `<id>.control/`; nadzorca sprawdza anulowanie ponownie zaraz po zapisaniu procesu i, gdy jest ustawione, od razu kończy całe drzewo procesów |

Drzewo procesów jest kończone jako grupa: `kill(-pgid)` w systemach Unix, obiekt
zadania (job object) w systemie Windows, tymi samymi wywołaniami co przy limicie
czasu. Wynik uruchomionego programu, który został anulowany, ma `killed_by: cancel`.

Anulowanie zadania, które się zakończyło, lub dwukrotne anulowanie zadania nie
jest błędem: odpowiedzią jest rekord zadania w jego bieżącym stanie. Klient
usuwa swoje powiadomienie o anulowaniu po odczytaniu odpowiedzi; nadzorca usuwa
każde powiadomienie o anulowaniu znalezione przy swoim uruchomieniu.

## 4. Żądania sterujące {#4-control-requests}

Zgłaszający zapisuje `<id>.control/<name>.request.json` z działaniem `stop`
(z `signal` i `grace`), `suspend`, `resume` lub `cancel`. Wątek zadania procesu
nadzorującego odczytuje katalog przy każdym odpytaniu (co 50 ms), usuwa plik
żądania, wykonuje działanie i zapisuje `<name>.reply.json` z `done` (rekord
zadania) lub `failed` (błąd). `cancel` nie wymaga odpowiedzi. Zgłaszający, który
nie może już usunąć swojego żądania, czeka na odpowiedź, którą proces nadzorujący
zapisuje po końcowym stanie zatrzymanego zadania; czas oczekiwania to czas
`grace` plus 60 s dla `stop` i 10 s dla pozostałych działań.

## 5. Zgodność {#5-compatibility}

Powiadomienie o anulowaniu jest dodatkowym plikiem; wersja protokołu pozostaje `1`.

| Klient | Nadzorca | Zachowanie |
|---|---|---|
| starszy | nowszy | klient nigdy nie zapisuje powiadomienia o anulowaniu; anulowanie po przejęciu żądania przez nadzorcę dociera do programu jak dotychczas, przez żądanie sterujące po uruchomieniu programu, a nadzorca od razu kończy drzewo, gdy znajdzie żądanie po zapisaniu procesu |
| nowszy | starszy | starszy nadzorca ignoruje pliki `*.cancel.json` (odczytuje tylko `*.request.json`); klient usuwa powiadomienie po odpowiedzi, a anulowanie dociera do programu przez żądanie sterujące |

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
