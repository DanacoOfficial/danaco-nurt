---
command: analyze.check
group: verify
summary: Sprawdza kod za pomocą kontrolerów typów, linterów, kontroli formatowania oraz analizy złożoności i duplikacji, a następnie zwraca werdykt.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# analyze.check

## Składnia {#synopsis}

```text
analyze.check <path>... [name: value]...
```

## Opis {#description}

Uruchamia analizatory skonfigurowane w projekcie oraz analizy wbudowane, łączy
ich ustalenia i zwraca werdykt w postaci nagłówka `analysis`, po którym
następują rekordy `finding`. Analiza jest zapisywana i można ją ponownie
odczytać za pomocą `analyze.show`.

Analizatory są wykrywane na podstawie plików konfiguracyjnych (`tsconfig.json`,
ESLint, Biome, Prettier, `pyproject.toml`, `ruff.toml`, `mypy.ini`, `go.mod`,
`Cargo.toml`) i uruchamiane z wyjściem czytelnym maszynowo. Program zainstalowany
w `node_modules/.bin` jest używany przed programem ze ścieżki wyszukiwania.
Kontrola, której narzędzie nie jest zainstalowane, ma stan `missing` i tworzy
notatkę `tool_missing`; polecenie nie kończy się błędem.

Złożoność i duplikacja są obliczane na podstawie danych usługi kodu. Bez tej
usługi obie kontrole mają stan `missing`.

Werdykt ma wartość `passed: false`, gdy ustalenie ma poziom ważności `fail_on`
lub wyższy. Parametry `min_severity` i `max_findings` ograniczają wyłącznie
zwracane ustalenia; liczby i werdykt obejmują wszystkie ustalenia.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `list<path>` | brak | Pliki lub katalogi do sprawdzenia. |
| `changed_only` | `bool` | `false` | Tylko pliki zmienione w repozytorium. |
| `checks` | `list<enum(types\|lint\|format\|complexity\|duplication)>` | brak | Kontrole do uruchomienia; po pominięciu wszystkie dostępne kontrole. |
| `include_tests` | `bool` | `true` | Uwzględnia pliki testów. |
| `complexity_max` | `int` | `15` | Najwyższa akceptowana złożoność cyklomatyczna. |
| `fail_on` | `enum(error\|warning)` | `error` | Najniższy poziom ważności, który przekreśla werdykt. |
| `min_severity` | `enum(error\|warning\|info)` | `info` | Najniższy poziom ważności zwracanych ustaleń. |
| `max_findings` | `int` | `100` | Maksymalna liczba zwracanych ustaleń. |
| `sample_lines` | `int` | `5` | Maksymalna liczba wierszy kodu źródłowego pokazywanych przy każdym ustaleniu. |
| `wait` | `duration` | `3s` | Czas oczekiwania, po którym analiza jest kontynuowana w tle jako zadanie. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `analysis`, `finding`, `job` |

Brak wejścia. Wyjście: jeden rekord `analysis`, a następnie rekordy `finding`
uporządkowane według poziomu ważności, kontroli, pliku i wiersza. Rekord zadania
jest zwracany, gdy analiza jest kontynuowana w tle.

Pole `duplication_percent` rekordu `analysis` zawiera udział wierszy kodu
w analizowanych plikach, które należą do zduplikowanego bloku, w obu kopiach,
w procentach z jednym miejscem po przecinku. Wiersz jest liczony raz, niezależnie
od liczby bloków, które go obejmują. Pole jest puste, gdy kontrola duplikacji nie
została uruchomiona.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `run` |
| Skutek | `process` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `user_program` |
| Warunki odwracalności | Pliki zmienione przez programy analizy są odwracalne, gdy ich poprzednia zawartość została zachowana; skutki poza systemem plików są nieodwracalne. |

## Przykłady {#examples}

Sprawdzenie całego projektu.

```nurt example
analyze.check
```

Sprawdzenie zmienionych plików pod kątem błędów i ostrzeżeń.

```nurt example
analyze.check changed_only: true fail_on: warning
```

Uruchomienie wyłącznie kontroli złożoności z niższym progiem.

```nurt example
analyze.check checks: [complexity] complexity_max: 10
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_ARG` | Nieznana kontrola lub wartość `fail_on`. | Użyj wartości z tabeli parametrów. |
| `E_EXTERNAL_MISSING` | Podano `changed_only` bez usługi zapytań o repozytorium. | Uruchom z `changed_only: false`. |
| `E_PROGRAM_NOT_FOUND` | Nie udało się uruchomić programu analizatora. | Zainstaluj narzędzie lub popraw jego ścieżkę. |

## Powiązane {#related}

`analyze.show`, `analyze.format`, `analyze.tools`.
