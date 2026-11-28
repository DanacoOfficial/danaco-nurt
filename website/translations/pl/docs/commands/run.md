---
command: run
group: core
summary: Uruchamia program bez powłoki; po upływie wait jest kontynuowany jako zadanie w tle.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# run

## Składnia {#synopsis}

```text
run <command> cwd: <path> [name: value]...
```

## Opis {#description}

`run` uruchamia program z argumentami z `command` jako listą. Powłoka nie bierze udziału: argumenty trafiają do programu dokładnie tak, jak zapisano, a składnia powłoki, taka jak `|`, `>`, `*` lub `$NAME`, nie ma znaczenia. Skrypt powłoki jest uruchamiany jako osobny program, na przykład `run ["bash", "-c", "make && make install"]`.

Pierwszy element `command` wskazuje program. Nazwa z katalogiem, taka jak `./tool` lub `/usr/bin/tool`, jest traktowana względem `cwd`. Każda inna nazwa jest wyszukiwana w `PATH` środowiska otrzymywanego przez program, więc `PATH` podane w `env` zaczyna obowiązywać; puste wpisy `PATH` są pomijane. W systemie Windows rozszerzenia z `PATHEXT` są próbowane po kolei po nazwie w podanej postaci, a nazwy zmiennych są porównywane bez uwzględniania wielkości liter.

W systemie Windows pliki wsadowe (`.bat`, `.cmd`) są uruchamiane przez `%SystemRoot%\System32\cmd.exe /e:ON /v:OFF /d /c`, z każdym argumentem ujętym w cudzysłów dla interpretera poleceń: argumenty ze spacjami lub metaznakami (`& | < > ^ ( ) ! "`) są ujmowane w cudzysłowy, `"` jest podwajany, a `%` jest zapisywany tak, aby `%NAME%` nie było rozwijane. Interpreter nigdy nie jest wyszukiwany w `PATH`. Kontrola statyczna wyświetla argumenty z metaznakami w `metacharacter_arguments`.

Środowisko jest budowane w trzech warstwach, z których każda nadpisuje wcześniejsze: środowisko konsoli (`inherit_env: all`), tabela `run.env_defaults` i `env`. Zmienna z `env` o wartości `null` jest usuwana; pusty tekst ustawia pustą wartość. Z `inherit_env: none` program otrzymuje tylko `run.env_defaults` i `env`; w systemie Windows zachowywane są także `SystemRoot`, `ComSpec` i `PATHEXT`, ponieważ bez nich programy nie uruchamiają się. Program podany nazwą jest wyszukiwany w `PATH` tego środowiska, więc w systemie Unix wymaga `PATH` w `env` (lub w `run.env_defaults`) przy `inherit_env: none`, w przeciwnym razie wynikiem jest `E_PROGRAM_NOT_FOUND`. Wynik wyświetla nazwy podane w `env` w `env_keys`; wartości nigdy nie pojawiają się w wyniku ani w dzienniku. Kontrola statyczna zgłasza zmienne z `run.env_defaults` w `env_defaults`.

Nie ma terminala: standardowe wejście jest puste (`/dev/null`, `NUL`), chyba że `stdin` podaje tekst, zapisywany do programu i następnie zamykany, lub ścieżkę, otwieraną jako wejście programu. Gdy program zamknie wejście przed zapisaniem całego tekstu, reszta jest odrzucana, a raport zawiera `stdin_closed_early`. Program działa we własnej grupie procesów (Windows: obiekt zadania); `timeout`, `idle_timeout`, `max_cpu` i `max_memory` dotyczą całego drzewa procesów.

Przetwarzanie wyjścia. Pełne wyjście jest przechowywane w plikach buforowych; poniższe parametry kształtują tylko odpowiedź, która domyślnie jest krótka. Każdy wiersz jest dekodowany (`output_encoding`), czyszczony (usuwane są sekwencje sterujące terminala, przerysowania przez powrót karetki, które zachowują tylko ostatni stan wiersza, oraz białe znaki na końcu), sprawdzany względem `summary` i `progress`, zwijany z powtórzeniami, gdy ustawiono `dedupe` (wartość domyślna: kolejne wiersze identyczne lub różniące się tylko liczbami, wersjami albo nazwami po słowie kroku, takim jak `Compiling`, stają się pierwszym z nich, a po nim ` (×N)`), wybierany przez `filter` z `context` wierszami wokół każdego dopasowania (albo, dla `show: errors` i dla `show: auto` po niepowodzeniu przy wyjściu dłuższym niż 20 wierszy, przez wbudowane wzorce wierszy błędów z 2 wierszami kontekstu), ograniczany do pierwszych `head` i ostatnich `tail` wierszy oraz obcinany do 400 znaków na wiersz (reszta staje się `… [+N characters]`). Tekst wszystkich strumieni razem mieści się następnie w `max_bytes` bajtów (domyślnie 4096, klucz `output.max_bytes`, `0` oznacza brak limitu), wypełnianych w tej kolejności: błędy znalezione w wyjściu znanego narzędzia, koniec standardowego wyjścia błędów, koniec standardowego wyjścia, początek wyjścia; koniec ma pierwszeństwo przed początkiem. `quiet_on_success` i `show: none` nie zwracają tekstu, ale zachowują liczniki, `summary_lines` i `progress`. `shown` zgłasza dla każdego strumienia wiersze zachowane od początku i od końca, wiersze pominięte między nimi oraz wiersze usunięte przez filtry.

Z `show: auto` i bez `head`, `tail` i `filter` uruchomienie kończące się kodem 0 zwraca ostatnie 5 wierszy swojego wyjścia i pole `digest`, jeden wiersz z kodem zakończenia, czasem oraz liczbą wierszy i bajtów wyjścia (dla znanego narzędzia także jego liczniki, na przykład `cargo: 0 errors, 3 warnings`). Nieudane uruchomienie zwraca pełny budżet.

Znane formaty. Po niepowodzeniu przy wyjściu dłuższym niż 20 wierszy `show: auto` rozpoznaje wyjście `cargo` i `rustc` (`error[E0308]` z `--> file:line:col`) oraz `pytest` (`FAILED file::test - message` z wierszem śladu stosu). Pole `diagnostics` zawiera wtedy `tool`, `errors` i `warnings` (liczby wszystkich) oraz `first`, czyli pierwszych 10 różnych błędów, każdy z jednowierszowymi polami `file`, `line`, `col`, `code`, `message` i `severity` (oraz `count`, gdy ten sam błąd zgłoszono w kilku miejscach); tekst zachowuje tylko ostatnie 10 wierszy wyjścia zamiast setek ostrzeżeń. Krótsze wyjście jest zwracane w całości i nie ma `diagnostics`. Wiersz `error: message` bez kodu i lokalizacji jest liczony jako błąd `cargo` tylko wtedy, gdy wyjście pokazuje inny jego znak.

Gdy odpowiedź pomija wiersze lub obcina wiersze, raport zawiera pole `continuation` (`id`, `unit: lines`, `shown`, `remaining`, `cut_lines`), a tekst kończy się wskazówką „more: next”. `next` zwraca pominięte wiersze z pliku wyjściowego w częściach wielkości pierwszej odpowiedzi, `next 200` zwraca 200 wierszy, `next all` wszystko, co pozostało, a `next full` wiersze obcięte przez odpowiedź, w całości; program nie jest uruchamiany ponownie. Numery wierszy są numerami pliku, więc wiersze zwinięte przez `dedupe` liczą się jako wiersze, które zastępują. Odpowiedź pokazująca wybór wierszy (`show: errors`) jest kontynuowana od pierwszego wiersza pliku. Plik wyjściowy pozostaje w rekordzie (`output_file`, `stdout_file`, `stderr_file`) i nadal można go odczytać poleceniami `read` i `search`.

`run` czeka na program do `wait`. Program, który kończy się w czasie `wait`, daje rekord `exec`. Program nadal działający po `wait` jest kontynuowany jako zadanie w tle, a wynikiem jest rekord `job` ze `state: running`; etapy po skutku są wtedy pomijane, a raport zawiera `post_stages_deferred`. `background: true` jest tym samym co `wait: 0s`; `wait: forever` czeka do `timeout`. Parametry, których program nie podaje, przyjmują wartości z kluczy konfiguracji `run.timeout`, `run.wait`, `run.idle_timeout`, `run.track_changes`, `run.parallel`, `output.head`, `output.tail` i `output.max_bytes`.

Zmiany wprowadzone przez program są wykrywane za pomocą `track_changes`. Tryb `auto` jest ograniczony tak, aby nigdy nie zatrzymał uruchomienia: nie skanuje korzenia systemu plików, nie wchodzi do katalogów innych systemów plików i rezygnuje, gdy obserwowane katalogi zawierają więcej niż 200 000 wpisów lub jedno skanowanie trwa dłużej niż 3 s. Program działa wtedy bez wykrywania, a raport zawiera notatkę `detection_scope` podającą przyczynę i wskazującą, co zrobić: wskazać w `watch` katalogi, które program zmienia, lub użyć `track_changes: scan`. Uruchomienie ze śledzeniem zawiera notatkę `detection_scope` wymieniającą obserwowane katalogi i katalogi innych systemów plików, do których nie wchodzono. Skanowania przed programem i po nim kończą się przy anulowaniu i przy `timeout`; skanowanie przed programem, które osiągnie `timeout`, kończy uruchomienie błędem `E_TIMEOUT`, a anulowane błędem `E_CANCELLED`. Skanowanie po programie może przekroczyć `timeout` o najwyżej 5 s, aby zmiany programu zakończonego przez limit zostały nadal zapisane; skanowanie przerwane wcześniej nie zapisuje zmian i informuje o tym w notatce. `run`, które nie zmienia żadnego pliku, nie zgłasza `nothing_matched`, ponieważ program nie jest etapem wybierającym cele.

Kod zakończenia różny od zera jest wynikiem, a nie błędem: stan to `ok`, `exec.exit_code` zawiera kod, a raport zawiera `exit_nonzero`. Użyj `|> assert exit_code: 0`, aby na nim zakończyć się niepowodzeniem. Program, który konsola kończy z powodu `timeout` lub `idle_timeout`, kończy się błędem `E_TIMEOUT`, program zakończony przez `max_cpu` lub `max_memory` błędem `E_LIMIT`, a program zakończony przez anulowanie błędem `E_CANCELLED`; błąd zawiera rekord `exec` w `details.exec` oraz `effect_applied: true`, ponieważ zmiany wprowadzone do tego czasu pozostają.

Z rekordami wejściowymi `path` każdego rekordu `entry` lub `match` staje się argumentem po `command`. Bez `batch` dla każdego rekordu działa jeden proces; z `batch: n` argumenty są dzielone na partie po najwyżej `n`. `parallel` uruchamia tyle partii naraz; wyniki zachowują kolejność partii. Wszystkie partie działają do końca, zanim wywołanie wróci, niezależnie od `wait`. Wynikiem jest jeden rekord `exec`: jego `exit_code` wynosi 0, gdy każda partia zakończyła się kodem 0, w przeciwnym razie jest kodem pierwszej nieudanej partii w kolejności partii; `batches` wyświetla numer, argumenty, kod zakończenia i czas działania każdej partii, a `failed_batches` liczy niepowodzenia. Po niepowodzeniu pozostałe partie nadal działają, chyba że ustawiono `bail: true`, które nie uruchamia kolejnych partii. Puste wejście nie uruchamia żadnego procesu. Gdy `command` używa pól rekordu wejściowego, polecenie jest obliczane dla każdego rekordu, a `batch` nie jest przyjmowane.

Podgląd (`preview: true`) rozwiązuje program i zwraca program, argumenty w postaci otrzymywanej przez program (`argv`; dla plików wsadowych wiersz poleceń interpretera), katalog roboczy, `launcher`, `env_keys`, `stdin`, `timeout`, `wait`, `watch` i `track_changes`, bez uruchamiania czegokolwiek.

Wiersz poleceń pliku wsadowego nie może zawierać przerwania wiersza: interpreter w tym miejscu kończy polecenie. Argument z `\r` lub `\n` dla pliku wsadowego oraz argument ze znakiem NUL dla dowolnego programu kończy się błędem `E_ARG`; `details.index` podaje pozycję w `command`, a `fix` przekazuje tekst zamiast tego na standardowe wejście.

Każde zadanie uruchomienia, partii lub `retry` emituje `job.started`, następnie `job.output` i `job.progress`, a potem jedno `job.finished`, także gdy zadanie jest kontynuowane w tle. Polecenie nie emituje `exec.started` ani `exec.finished`: te zdarzenia obejmują cały program i pochodzą z silnika.

**Uprawnienia.** `privilege: elevated` uruchamia program z uprawnieniami administratora, `as_user: "NAME"` jako inne konto; `sudo`, `doas` i `run0` nie są zapisywane na liście argumentów (lista zaczynająca się od `sudo` jest poprawiana na to samo `run` z `privilege: elevated`). Pliki zmieniane przez program są zgłaszane, ale nie są zachowywane: wpis jest nieodwracalny, a raport informuje o tym przed zmianą.

```nurt
run ["systemctl", "daemon-reload"] cwd: "/" privilege: elevated
```

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `command` | `list<text>` | wymagany | Program i argumenty; pierwszy element to program. |
| `cwd` | `path` | wymagany | Katalog roboczy programu. |
| `env` | `map<text>` | none | Zmienne środowiskowe do ustawienia. |
| `inherit_env` | `enum(all\|none)` | `all` | Dziedziczy środowisko konsoli lub zaczyna od pustego. |
| `stdin` | `text \| path` | none | Tekst lub plik podany na standardowe wejście; w przeciwnym razie wejście jest puste. |
| `timeout` | `duration` | `120s` | Limit czasu programu. |
| `idle_timeout` | `duration` | `60s` | Limit czasu bez wyjścia. |
| `wait` | `duration \| enum(forever)` | `3s` | Czas oczekiwania na program, zanim będzie kontynuowany jako zadanie; forever czeka do timeout. |
| `background` | `bool` | `false` | Uruchamia od razu jako zadanie, tak samo jak wait: 0s. |
| `merge_stderr` | `bool` | `false` | Scala standardowe wyjście błędów ze standardowym wyjściem. |
| `show` | `enum(auto\|both\|stdout\|stderr\|errors\|summary\|none)` | `auto` | Wyjście zwracane w wyniku; pełne wyjście jest zawsze przechowywane w pliku. |
| `head` | `int` | `20` | Liczba zwracanych pierwszych wierszy wyjścia. |
| `tail` | `int` | `40` | Liczba zwracanych ostatnich wierszy wyjścia. |
| `max_bytes` | `int` | `4096` | Liczba bajtów tekstu wyjścia zwracanych we wszystkich strumieniach razem; 0 oznacza brak limitu. |
| `filter` | `regex` | none | Zwraca tylko pasujące wiersze wyjścia. |
| `context` | `int` | `0` | Liczba wierszy kontekstu wokół wierszy pasujących do filter. |
| `dedupe` | `bool` | `true` | Zwija powtórzone wiersze wyjścia, także wiersze różniące się tylko liczbami. |
| `quiet_on_success` | `bool` | `false` | Nie zwraca wyjścia, gdy kod zakończenia wynosi 0. |
| `output_encoding` | `text` | `"auto"` | Kodowanie wyjścia programu; auto wykrywa je automatycznie. |
| `watch` | `list<path>` | none | Katalogi, w których wykrywane są zmiany wprowadzone przez program. |
| `track_changes` | `enum(auto\|scan\|events\|none)` | `auto` | Metoda wykrywania zmian wprowadzonych przez program. |
| `max_cpu` | `duration` | none | Limit czasu procesora. |
| `max_memory` | `size` | none | Limit pamięci. |
| `priority` | `enum(low\|normal\|high)` | `normal` | Priorytet szeregowania. |
| `label` | `text` | none | Krótka nazwa zadania. |
| `description` | `text` | none | Opis zadania. |
| `progress` | `regex` | none | Wzorzec wierszy wyjścia zgłaszających postęp. |
| `summary` | `list<regex>` | none | Wzorce, których najnowsze dopasowania tworzą wiersz stanu. |
| `batch` | `int` | none | Uruchamia program raz dla każdej partii tylu argumentów wejściowych. |
| `parallel` | `int` | `1` | Liczba partii uruchamianych jednocześnie; wyniki zachowują kolejność partii. |
| `bail` | `bool` | `false` | Przestaje uruchamiać partie po pierwszej nieudanej partii. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none`, `entry`, `match` | `exec`, `job` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `run` |
| Skutek | `process` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `user_program` |
| Warunki odwracalności | Zmiany plików objętych watch są odwracalne, gdy ich poprzednia zawartość została zachowana; skutki poza systemem plików są nieodwracalne. |

Zmiany plików objętych `watch` są zapisywane w dzienniku i są odwracalne, gdy ich poprzednia zawartość została zachowana; skutki poza systemem plików są nieodwracalne. Uruchomienie, które nie zmienia żadnego pliku, nie tworzy wpisu dziennika.

Wiersz poleceń pasujący do wbudowanej tabeli programów ze skutkami poza komputerem (na przykład `git push` lub `kubectl apply`, także za opakowaniami takimi jak `sudo` lub `env`) albo do wzorca z `run.irreversible_patterns` wyświetla dopasowane reguły w `exec.external_effect`, a raport oznacza wywołanie jako `irreversible: external_effect`. Klasyfikacja opisuje skutek; nigdy nie uniemożliwia uruchomienia.

## Przykłady {#examples}

Uruchomienie testów projektu i wyświetlenie tylko wierszy błędów:

```nurt example
run ["cargo", "test"] cwd: "/home/user/app" show: errors head: 5 tail: 10
```

Uruchomienie kompilacji w tle z etykietą:

```nurt example
run ["npm", "run", "build"] cwd: "/home/user/app" background: true label: "build"
```

Sformatowanie plików Go znalezionych w projekcie, po 20 plików na proces, dwa procesy naraz:

```nurt example
find "/home/user/app" glob: "**/*.go" |> run ["gofmt", "-w"] batch: 20 parallel: 2 cwd: "/home/user/app"
```

Przekazanie tekstu na standardowe wejście, z własnym środowiskiem:

```nurt example
run ["sort"] cwd: "/home/user" stdin: "b\na\n" inherit_env: none env: {"LC_ALL": "C", "PATH": "/usr/bin:/bin"}
```

Zakończenie niepowodzeniem, gdy program kończy się kodem innym niż zero:

```nurt example
run ["go", "build", "./..."] cwd: "/home/user/app" |> assert exit_code: 0
```

Odpowiedź, która pomija wiersze, kończy się wskazówką „more: next” i zawiera pole `continuation`; polecenie `next` zwraca kolejną część bez ponownego uruchamiania polecenia.

```text
exit 1  3000 lines  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_PROGRAM_NOT_FOUND` | Programu nie ma w `PATH` lub wskazany plik nie istnieje; `candidates` wyświetla do pięciu podobnych nazw ze ścieżki wyszukiwania. | Zaakceptuj kandydata, podaj pełną ścieżkę lub dodaj katalog do `PATH` w `env`. |
| `E_SPAWN` | Nie udało się uruchomić programu: katalog roboczy nie istnieje lub nie jest katalogiem, plik jest katalogiem lub nie ma uprawnienia do wykonania albo format jest nieznany. | Podaj istniejący `cwd`; nadaj uprawnienie do wykonania lub uruchom plik przez jego interpreter. |
| `E_ARG` | Argumentu lub zmiennej nie można przekazać bez zmiany ich znaczenia: znak NUL, przerwanie wiersza w argumencie pliku wsadowego, `=` w nazwie zmiennej. | Użyj `fix`, które przekazuje tekst na standardowe wejście. |
| `E_MISSING_PARAM` | Nie podano ani `cwd`, ani `base`. | Dodaj `cwd`. |
| `E_PATH_RELATIVE` | `cwd` jest względne, a `base` nie jest podane. | Podaj bezwzględne `cwd`. |
| `E_INVALID_VALUE` | `batch` lub `parallel` jest mniejsze niż 1 albo `batch` jest użyte z `command` obliczanym dla każdego rekordu. | Podaj liczbę 1 lub większą; usuń `batch`. |
| `E_NOT_FOUND` | Plik `stdin` nie istnieje. | Podaj istniejący plik lub tekst. |
| `E_TIMEOUT` | Program osiągnął `timeout` lub `idle_timeout`; `details.exec` zawiera rekord. | Podnieś limit lub uruchom program z `background: true`. |
| `E_LIMIT` | Program przekroczył `max_cpu` lub `max_memory`. | Podnieś limit. |
| `E_CANCELLED` | Wywołanie zostało anulowane w trakcie działania programu. | Uruchom wywołanie ponownie. |

## Powiązane {#related}

`jobs`, `stop`, `retry`, `suspend`, `resume`, `assert`, `verify`, `undo`, `test.run`, `analyze.check`.
