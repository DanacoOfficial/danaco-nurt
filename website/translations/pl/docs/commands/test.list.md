---
command: test.list
group: verify
summary: Wylistowuje wcześniejsze przebiegi testów.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# test.list

## Składnia {#synopsis}

```text
test.list [name: value]...
```

## Opis {#description}

Zwraca nagłówki `test_run` zapisanych przebiegów, od najnowszego. Historia
przechowuje liczbę przebiegów ustawioną przez `verify.history_max` (wartość
domyślna `50`); starsze przebiegi są usuwane, gdy zapisywany jest nowy
przebieg. Zapisany przebieg, którego nie można odczytać, jest pomijany na
liście i zgłaszany w notatkach wyniku jako `source_unavailable`, ze ścieżką
jego pliku i radą usunięcia pliku; pozostałe przebiegi są wylistowane.
`test.show` tego przebiegu nadal kończy się błędem nieczytelnego wpisu.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `limit` | `int` | `20` | Maksymalna liczba przebiegów, od najnowszego. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `none` | `test_run` |

Polecenie nie przyjmuje wejścia. Daje rekordy `test_run`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wylistowanie pięciu najnowszych przebiegów.

```nurt example
test.list limit: 5
```

Wylistowanie przebiegów, które zakończyły się niepowodzeniem.

```nurt example
test.list |> filter status == "failed"
```

Pokazanie identyfikatora i odcisku najnowszego przebiegu.

```nurt example
test.list limit: 1 |> select id fingerprint
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | `limit` jest ujemny albo za duży. | Podaj liczbę od 0 do 4294967295. |

## Powiązane {#related}

`test.run`, `test.show`, `test.failed`.
