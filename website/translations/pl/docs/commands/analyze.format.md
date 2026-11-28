---
command: analyze.format
group: verify
summary: Formatuje pliki źródłowe za pomocą formatera skonfigurowanego dla danego języka.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# analyze.format

## Składnia {#synopsis}

```text
analyze.format <path>... [name: value]...
```

## Opis {#description}

Znajduje pliki, które skonfigurowane formatery by zmieniły, a następnie
uruchamia formatery dokładnie na tych plikach. Formaterami są Prettier, Biome,
Ruff, `gofmt` i `rustfmt`. Zbiór plików jest ustalany przed uruchomieniem
jakiegokolwiek formatera: każdy formater jest najpierw uruchamiany w trybie
kontroli na żądanych plikach.

Wynik wymienia pliki i pokazuje do `sample_lines` zmienionych wierszy na plik
jako wiersze `-old` i `+new`.

Pliki zmienione poza ustalonym zbiorem są wykrywane tak jak dla `run`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `list<path>` | brak | Pliki lub katalogi do sformatowania. |
| `changed_only` | `bool` | `false` | Tylko pliki zmienione w repozytorium. |
| `sample_lines` | `int` | `12` | Maksymalna liczba zmienionych wierszy pokazywanych na plik, od 1 do 200. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

Brak wejścia. Wyjście: raport skutków z jedną zmianą na każdy sformatowany plik.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `process` |
| Odwracalność | `reversible` |
| Implementacja | `user_program` |
| Warunki odwracalności | Poprzednia zawartość każdego pliku przekazanego do formatera jest przechowywana w dzienniku. |

## Przykłady {#examples}

Sformatowanie plików zmienionych w repozytorium.

```nurt example
analyze.format changed_only: true
```

Sformatowanie jednego katalogu z pokazaniem większej liczby zmienionych wierszy.

```nurt example
analyze.format "/home/user/app/src" sample_lines: 40
```

Sformatowanie dwóch katalogów.

```nurt example
analyze.format "/home/user/app/src" "/home/user/app/tests"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_SPAWN` | Formater zakończył się niepowodzeniem. | Uruchom `analyze.check checks: [format]` i popraw zgłoszone błędy składni. |
| `E_PROGRAM_NOT_FOUND` | Nie udało się uruchomić programu formatera. | Zainstaluj formater lub popraw jego ścieżkę. |

## Powiązane {#related}

`analyze.check`, `analyze.tools`.
