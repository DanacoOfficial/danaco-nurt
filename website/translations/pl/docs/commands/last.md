---
command: last
group: core
summary: Zachowuje ostatnie rekordy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# last

## Składnia {#synopsis}

```text
any |> last <count>
```

## Opis {#description}

`last` przekazuje dalej ostatnie `count` rekordów wejścia. Etap odczytuje całe
wejście i przechowuje jednocześnie nie więcej niż `count` rekordów; gdy
`count` jest większe niż limit rekordów żądania, a wejście jest dłuższe niż
limit, program kończy działanie błędem `E_LIMIT`.

Kolejność, nagłówek i wskazówka prezentacji strumienia zostają zachowane.
Wejście zawierające mniej rekordów niż `count` jest przekazywane w całości.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `count` | `int` | wymagany | Liczba rekordów do zachowania. |

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

Zachowanie ostatniego pliku z listy:

```nurt example
find "/home/user/project" |> last 1
```

Wyświetlenie ostatnich dwóch wierszy dziennika:

```nurt example
lines "/home/user/app.log" |> last 2
```

Zachowanie wszystkich plików, gdy jest ich mniej niż dziesięć:

```nurt example
find "/home/user/project" |> last 10
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | brak `count` | wpisz liczbę po `last` |
| `E_INVALID_VALUE` | `count` jest ujemne | użyj 0 lub większej liczby |
| `E_LIMIT` | należy przechować więcej rekordów, niż dopuszcza limit | zmniejsz `count` lub zawęź wejście |

## Powiązane {#related}

`first`, `skip`, `sort`.
