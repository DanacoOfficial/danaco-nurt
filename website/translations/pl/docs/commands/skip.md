---
command: skip
group: core
summary: Pomija pierwsze rekordy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# skip

## Składnia {#synopsis}

```text
any |> skip <count>
```

## Opis {#description}

`skip` pomija pierwsze `count` rekordów wejściowych i przekazuje dalej
pozostałe. Etap jest leniwy: odczytuje pomijane rekordy pojedynczo, a każdy
kolejny rekord przekazuje dalej, gdy poprosi o niego następny etap. Błąd
źródła wśród pomijanych rekordów nadal zatrzymuje program.

Kolejność, nagłówek i wskazówka prezentacji strumienia zostają zachowane.
Wejście zawierające nie więcej niż `count` rekordów daje puste wyjście.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `count` | `int` | wymagany | Liczba rekordów do pominięcia. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `any` | `same` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `transform` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Pominięcie pierwszego pliku listingu:

```nurt example
find "/home/user/project" |> skip 1
```

Pominięcie wiersza nagłówka pliku CSV:

```nurt example
lines "/home/user/data.csv" |> skip 1
```

Pominięcie większej liczby rekordów, niż zawiera wejście, nie daje żadnych rekordów:

```nurt example
find "/home/user/project" |> skip 5
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | brak `count` | wpisz liczbę po `skip` |
| `E_INVALID_VALUE` | `count` jest ujemne | użyj 0 lub większej liczby |

## Powiązane {#related}

`first`, `last`.
