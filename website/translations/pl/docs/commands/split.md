---
command: split
group: text
summary: Dzieli wiersze na kolumny według separatora.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# split

## Składnia {#synopsis}

```text
line |> split <delimiter> [name: value]...
```

## Opis {#description}

`split` dzieli `text` każdego rekordu `line` w miejscu `delimiter` i zwraca
rekord `row`, którego `cells` są częściami, jako tekst. Separator tekstowy
jest dopasowywany dosłownie; wyrażenie regularne pasuje do dowolnego
separatora, który opisuje, na przykład `/\s+/` dla ciągów białych znaków.

Komórki są nazywane kolejno według `names`. Części ponad liczbę nazw są
nazywane `c<n>` według ich pozycji od 1, a nazwy ponad liczbę części
otrzymują `null`. Bez `names` komórki mają nazwy `c1`, `c2` i tak dalej. Wiersz
zachowuje `path` oraz numer wiersza `no` swojego wiersza. Etap jest leniwy i
zachowuje nagłówek strumienia.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `delimiter` | `text \| regex` | wymagany | Tekst lub wzorzec separatora. |
| `names` | `list<text>` | brak | Nazwy kolumn. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `line` | `row` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `transform` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Podział wierszy CSV na ponumerowane kolumny:

```nurt example
lines "/home/user/data.csv" |> split ","
```

Podział wierszy CSV po nagłówku na nazwane kolumny:

```nurt example
lines "/home/user/data.csv" |> skip 1 |> split "," names: ["id", "name"]
```

Podział kolumn rozdzielonych białymi znakami:

```nurt example
lines "/home/user/table.txt" |> split /\s+/ names: ["user", "shell"]
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | separator jest pustym tekstem | podaj co najmniej jeden znak |
| `E_REGEX` | wzorzec separatora się nie kompiluje | popraw wzorzec |
| `E_TYPE` | rekordy wejściowe nie są rekordami `line` albo `names` nie jest listą tekstów | odczytaj plik za pomocą `lines`; zapisz nazwy jako `["a", "b"]` |

## Powiązane {#related}

`parse`, `lines`, `skip`.
