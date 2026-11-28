---
command: test.run
group: verify
summary: Uruchamia testy projektu i zwraca typowane wyniki wraz z odciskiem stanu kodu.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# test.run

## Składnia {#synopsis}

```text
test.run [<path>] [name: value]...
```

## Opis {#description}

Uruchamia framework testowy projektu w `path` i zwraca jeden nagłówek
`test_run` oraz jeden rekord `test_result` na każdy test. Framework jest
uruchamiany jako program bez powłoki. Jego raport czytelny maszynowo jest
zapisywany w katalogu stanu, a nie w projekcie.

Przy `framework: auto` framework jest wykrywany z plików projektu w tej
kolejności: `bun` (`bunfig.toml`, `bun.lock`, skrypt `test`, który uruchamia
`bun test`), `vitest`, `jest`, `npm` (skrypt `test` inny niż symbol zastępczy
zapisany przez `npm init`), `pytest`, `go` (`go.mod`) i `cargo`
(`Cargo.toml`). Katalog zawierający pliki `*.test.*` lub `*.spec.*` i żaden
inny znacznik jest uruchamiany za pomocą `bun`. `command` zastępuje polecenie
frameworka; bez `framework` jego wyjście jest czytane jako TAP i jako wiersze
oznaczone `(pass)`, `(fail)`, `(skip)` lub `(todo)`.

`targets` i `filter` są tłumaczone na opcje frameworka, a `bail` na jego opcję
przerwania przy pierwszym niepowodzeniu. Cel zaczynający się od `-` jest
odrzucany; opcje frameworka podaje się przez `command`. Framework `npm` jest
zapisywany jako `custom` w `test_run`.

Identyfikator testu jest stały między przebiegami: plik, zestaw i nazwa
połączone przez `::`. Powtórzony identyfikator otrzymuje przyrostek `#2`,
`#3`. Przebieg jest porównywany z wcześniejszymi przebiegami tego samego
katalogu. Nagłówek zawiera wtedy `baseline` (przebieg z największą liczbą
wspólnych testów), `regressions` (testy, które przechodziły, a teraz zawodzą)
i `fixed` (testy, które zawodziły, a teraz przechodzą), każde tylko wtedy, gdy
ma zastosowanie. Przy `slowest: n` nagłówek zawiera `slowest`, czyli `n`
najwolniejszych testów jako `id: <time>ms`.

`failures` ogranicza, ile testów zakończonych niepowodzeniem zachowuje w
wyjściu swoje `message`, `stack`, `stdout` i `stderr`; `message_lines` przycina
każdy z tych tekstów do tej liczby wierszy. Zapisany przebieg zachowuje pełne
teksty.

Przy `coverage: true` framework mierzy pokrycie: `--coverage` dla bun, vitest
i jest, `pytest-cov` dla pytest, `-coverprofile` dla go oraz `cargo llvm-cov`
zamiast `cargo test` dla cargo. `coverage_file` wskazuje raport, który testy
tworzą same; jest on odczytywany po przebiegu, a jego format (LCOV, Cobertura,
Istanbul, Go, LLVM) jest wykrywany z jego zawartości. Wynikiem jest pole
`coverage` rekordu `test_run`.

`fingerprint` nagłówka to `sha256:` i skrót posortowanej listy ścieżek plików
i skrótów ich zawartości. W repozytorium git plikami są pliki śledzone i
zmienione pliki niezśledzone; w przeciwnym razie są to pliki znalezione według
reguł ignorowania katalogu. Nazwy w `run.preimage_exclude` są pomijane.
Obejmowanych jest co najwyżej 20 000 plików.

Gdy przebieg trwa nadal po `wait`, jest kontynuowany jako zadanie, a
polecenie zwraca rekord `job`. Przebieg testów jest zapisywany po zakończeniu
zadania.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | brak | Katalog projektu. |
| `framework` | `enum(auto\|bun\|vitest\|jest\|npm\|pytest\|go\|cargo)` | `auto` | Framework testowy; auto wykrywa go z projektu. |
| `targets` | `list<text>` | brak | Pliki testowe, pakiety lub identyfikatory do uruchomienia. |
| `filter` | `text` | brak | Tylko testy, których nazwa pasuje do tego filtra. |
| `bail` | `bool` | `false` | Zatrzymuje się na pierwszym teście zakończonym niepowodzeniem. |
| `command` | `list<text>` | brak | Polecenie testowe uruchamiane zamiast wykrytego polecenia frameworka. |
| `coverage` | `bool` | `false` | Zbiera pokrycie kodu i zwraca je wraz z przebiegiem. |
| `coverage_file` | `path` | brak | Raport pokrycia w formacie LCOV, Cobertura lub Go tworzony przez testy; pominięty jest wykrywany z frameworka. |
| `timeout` | `duration` | `30m` | Limit czasu przebiegu testów. |
| `wait` | `duration` | `3s` | Czas oczekiwania, zanim przebieg będzie kontynuowany w tle jako zadanie. |
| `slowest` | `int` | `0` | Liczba zgłaszanych najwolniejszych testów. |
| `failures` | `int` | `10` | Maksymalna liczba testów zakończonych niepowodzeniem zgłaszanych ze szczegółami. |
| `message_lines` | `int` | `5` | Maksymalna liczba wierszy komunikatu na test zakończony niepowodzeniem. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `none` | `test_run`, `test_result`, `job` |

Polecenie nie przyjmuje wejścia. Daje nagłówek `test_run`, po którym
następują rekordy `test_result`, albo rekord `job`, gdy przebieg jest
kontynuowany w tle.

Status przebiegu to `passed`, `failed`, `error` (framework nie wystartował
albo kod się nie skompilował), `timed_out` lub `no_tests`. Gdy przebieg kończy
się błędem i bez wyników testów, wynik z identyfikatorem `(run)` zawiera
koniec wyjścia. Plik testowy, którego nie można załadować, i pakiet Go, który
się nie buduje, są zgłaszane jako wyniki ze statusem `error`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `run` |
| Skutek | `process` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `user_program` |
| Warunki odwracalności | Pliki zmienione przez programy testowe są odwracalne, gdy ich poprzednia zawartość została zachowana; skutki poza systemem plików są nieodwracalne. |

## Przykłady {#examples}

Uruchomienie testów bieżącego projektu i pokazanie tylko tych, które zawiodły.

```nurt example
test.run |> filter status == "failed"
```

Uruchomienie jednego pliku, przy czym pierwsze niepowodzenie zatrzymuje przebieg.

```nurt example
test.run framework: bun targets: ["src/sum.test.ts"] bail: true
```

Uruchomienie projektu Go z pokryciem i pokazanie pokrycia przebiegu.

```nurt example
test.run framework: go coverage: true |> select coverage
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_TEST_NO_FRAMEWORK` | Nie rozpoznano żadnego frameworka. | Wskaż framework lub podaj `command:`. |
| `E_EXTERNAL_MISSING` | Program testowy nie jest zainstalowany. | Zainstaluj go lub podaj `command:`. |
| `E_OUTPUT_PARSE` | Nie można odczytać raportu ani wyjścia. | Uruchom z `command:`, aby zobaczyć wyjście; błąd zawiera rekord wykonania. |
| `E_INVALID_VALUE` | Cel wygląda jak opcja. | Przekaż opcje przez `command:`. |

## Powiązane {#related}

`test.failed`, `test.show`, `test.list`, `run`, `jobs`.
