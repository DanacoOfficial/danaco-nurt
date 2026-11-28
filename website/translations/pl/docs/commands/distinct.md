---
command: distinct
group: text
summary: Zachowuje jeden rekord dla każdej odrębnej wartości wraz z liczbą wystąpień.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# distinct

## Składnia {#synopsis}

```text
any |> distinct <by>...
```

## Opis {#description}

`distinct` zachowuje pierwszy rekord dla każdej odrębnej wartości pól z `by`
i dodaje pole `count` z liczbą rekordów, które miały tę wartość. Bez `by`
rekordy są odrębne ze względu na swój rodzaj i wszystkie swoje pola. Wartości
są porównywane według porządku całkowitego wartości.

Pierwsze wystąpienia zachowują kolejność z wejścia, więc porządek zadeklarowany
przez źródło pozostaje ważny. `distinct` odczytuje całe wejście; liczba
odrębnych rekordów jest ograniczona limitem rekordów żądania. Puste wejście
daje puste wyjście.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `by` | `list<field>` | brak | Pola, które czynią rekordy odrębnymi; wszystkie pola, gdy pominięty. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `any` | `same` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `transform` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Zachowanie jednego pliku dla każdego rozszerzenia:

```nurt example
find "/home/user/project" |> distinct extension
```

Zliczenie powtarzających się wierszy pliku:

```nurt example
lines "/home/user/words.txt" |> distinct text
```

Wyświetlenie plików z dopasowaniami po jednym razie:

```nurt example
search /error/ "/home/user/logs" |> select path |> distinct
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_UNKNOWN_FIELD` | pola nie ma w rekordach; `expected` wymienia ich pola | użyj jednego z wymienionych pól |
| `E_LIMIT` | odrębnych rekordów jest więcej niż limit | najpierw zawęź wejście |

## Powiązane {#related}

`group`, `select`, `sort`.
