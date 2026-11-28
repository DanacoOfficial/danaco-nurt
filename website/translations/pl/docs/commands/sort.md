---
command: sort
group: core
summary: Sortuje rekordy według pól.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# sort

## Składnia {#synopsis}

```text
any |> sort <by>... [name: value]...
```

## Opis {#description}

`sort` porządkuje rekordy według jednego lub kilku kluczy, od pierwszego
klucza. Kluczami są pola lub wyrażenia. Wartości są porównywane według
całkowitego porządku wartości: wartości różnych typów są porządkowane według
typu (najpierw `null`, potem `bool`, liczby, rozmiary, czasy trwania, chwile,
tekst, ścieżki i pozostałe typy), więc pole o mieszanych typach sortuje się
deterministycznie. Tekst jest porównywany według punktów kodowych Unicode.

Sortowanie jest stabilne: rekordy o równych kluczach zachowują kolejność
wejścia, także przy `order: desc`. `sort` odczytuje całe wejście i przechowuje
je w pamięci w granicach limitu rekordów żądania; większe wejście zatrzymuje
program z błędem `E_LIMIT`. Gdy źródło już produkuje rekordy w żądanej
kolejności, strumień przechodzi bez zmian. Wyjście deklaruje swoją kolejność,
więc następny `sort` po tych samych polach nie sortuje ponownie. Puste wejście
daje puste wyjście.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `by` | `list<field>` | wymagany | Pola, według których sortować, w kolejności priorytetu. |
| `order` | `enum(asc\|desc)` | `asc` | Kierunek sortowania. |

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

Sortowanie plików od największego:

```nurt example
find "/home/user/project" |> sort size order: desc
```

Sortowanie według rozszerzenia, a w obrębie rozszerzenia według rozmiaru:

```nurt example
find "/home/user/project" |> sort extension size
```

Sortowanie wierszy pliku alfabetycznie:

```nurt example
lines "/home/user/data.txt" |> sort text
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | nie podano żadnego klucza | wskaż co najmniej jedno pole |
| `E_UNKNOWN_FIELD` | klucz wskazuje pole, którego rekordy nie mają; `expected` wylicza ich pola | użyj jednego z wymienionych pól |
| `E_UNKNOWN_VALUE` | `order` nie jest ani `asc`, ani `desc` | wpisz `order: asc` lub `order: desc` |
| `E_LIMIT` | wejście zawiera więcej rekordów niż limit | najpierw zawęź wejście za pomocą `filter` |

## Powiązane {#related}

`first`, `last`, `group`, `distinct`.
