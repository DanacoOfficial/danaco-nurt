---
command: first
group: core
summary: Zachowuje pierwsze rekordy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# first

## Składnia {#synopsis}

```text
any |> first <count>
```

## Opis {#description}

`first` przekazuje dalej pierwsze `count` rekordów wejścia i odrzuca resztę.
Etap odczytuje z wejścia nie więcej niż `count` rekordów, więc źródło
zatrzymuje się po ich odczytaniu; `first 0` nie odczytuje niczego. Planer
przekazuje liczbę źródłom potoku, które mogą zakończyć działanie wcześniej.

Kolejność, nagłówek i wskazówka prezentacji strumienia są zachowane. Wejście
z mniejszą liczbą rekordów niż `count` jest przekazywane w całości.

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

Zachowanie dwóch pierwszych plików:

```nurt example
find "/home/user/project" |> first 2
```

Zachowanie największego pliku:

```nurt example
find "/home/user/project" |> sort size order: desc |> first 1
```

Wyświetlenie dwóch pierwszych wierszy dziennika:

```nurt example
lines "/home/user/app.log" |> first 2
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | brakuje `count` | zapisz liczbę po `first` |
| `E_INVALID_VALUE` | `count` jest ujemne | użyj 0 lub większej liczby |
| `E_TYPE` | `count` nie jest liczbą całkowitą | zapisz liczbę całkowitą, na przykład `first 20` |

## Powiązane {#related}

`last`, `skip`, `sort`.
