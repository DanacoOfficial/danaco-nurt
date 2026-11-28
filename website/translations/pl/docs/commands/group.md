---
command: group
group: text
summary: Grupuje rekordy według pola lub wyrażenia i zlicza każdą grupę.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# group

## Składnia {#synopsis}

```text
any |> group <by> [name: value]...
```

## Opis {#description}

`group` oblicza `by` dla każdego rekordu i zwraca jeden rekord `group` dla
każdej odrębnej wartości, z polami `key` (wartość) i `count` (liczba rekordów).
Przy `keep_items: true` pole `items` zawiera rekordy grupy w kolejności
wejścia. Wartości są porównywane według pełnego porządku wartości.

Grupy są uporządkowane według klucza, więc wynik nie zależy od kolejności
wejścia; wyjście deklaruje ten porządek. `group` odczytuje całe wejście; liczba
grup, a przy `keep_items` liczba przechowywanych rekordów, jest ograniczona
limitem rekordów żądania. Puste wejście nie daje żadnych grup.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `by` | `field \| expression` | wymagany | Pole lub wyrażenie, którego wartość tworzy grupy. |
| `keep_items` | `bool` | `false` | Dołącza rekordy każdej grupy. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `any` | `group` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `transform` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Zlicz pliki według rozszerzenia:

```nurt example
find "/home/user/project" |> group extension
```

Pogrupuj pliki według rozszerzenia i zachowaj je w każdej grupie:

```nurt example
find "/home/user/project" |> group extension keep_items: true
```

Podziel pliki na małe i duże:

```nurt example
find "/home/user/project" |> group size > 2kb
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | brakuje `by` | wskaż pole lub zapisz wyrażenie |
| `E_UNKNOWN_FIELD` | `by` wskazuje pole, którego rekordy nie mają; `expected` wymienia ich pola | użyj jednego z wymienionych pól |
| `E_LIMIT` | więcej grup lub rekordów niż limit | zawęź wejście albo pomiń `keep_items` |

## Powiązane {#related}

`summarize`, `distinct`, `sort`, `count`.
