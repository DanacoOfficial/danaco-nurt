---
command: summarize
group: text
summary: Oblicza liczbę, sumę, minimum, maksimum i średnią pól, opcjonalnie dla każdej grupy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# summarize

## Składnia {#synopsis}

```text
any |> summarize <fields>... [name: value]...
```

## Opis {#description}

`summarize` zwraca jeden rekord `summary` na każde pole z `fields`, a gdy
podano `by`, także na każdą grupę, z polami `field`, `count`, `sum`, `min`,
`max` i `average`; przy `by` rekord zaczyna się od wartości grupy pod nazwą
pola `by`.

`count` jest liczbą wartości innych niż `null`; wartości `null` są
pomijane. Liczby całkowite i liczby sumują się do liczby całkowitej lub
liczby, a średnia jest liczbą. Rozmiary i czasy trwania sumują się i uśredniają
we własnej jednostce, zaokrąglone do pełnych bajtów i milisekund. Dla tekstu
i innych typów oraz dla pól o mieszanych typach `sum` i `average` mają
wartość `null`, a `min` i `max` wynikają z całkowitego porządku wartości.

Bez `fields` każda grupa daje jeden rekord z liczbą jej rekordów w `count`.
Grupy są porządkowane według ich wartości. Puste wejście bez `by` daje jeden
rekord na pole z `count` równym 0.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `fields` | `list<field>` | brak | Pola liczbowe do podsumowania. |
| `by` | `field` | brak | Pole, którego wartość tworzy grupy. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `any` | `summary` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `transform` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Obliczenie łącznego i średniego rozmiaru plików projektu:

```nurt example
find "/home/user/project" |> summarize size
```

Obliczenie statystyk rozmiaru dla każdego rozszerzenia:

```nurt example
find "/home/user/project" |> summarize size by: extension
```

Zliczenie plików według rozszerzenia:

```nurt example
find "/home/user/project" |> summarize by: extension
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_UNKNOWN_FIELD` | pola nie ma w rekordach; `expected` wylicza ich pola | użyj jednego z wymienionych pól |
| `E_LIMIT` | więcej grup niż limit | grupuj według pola o mniejszej liczbie wartości |

## Powiązane {#related}

`group`, `count`, `distinct`.
