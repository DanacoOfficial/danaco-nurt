---
command: count
group: core
summary: Zlicza rekordy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# count

## Składnia {#synopsis}

```text
any |> count
```

## Opis {#description}

`count` odczytuje całe wejście i podaje liczbę rekordów jako `int`.
Rekordy nie są przechowywane w pamięci. Puste wejście daje 0.

`count` zlicza rekordy plików przed skutkiem oraz rekordy skutku po nim
(etap `5a`), na przykład rekordy `exec` uruchomionego programu.

## Parametry {#parameters}

Polecenie nie ma parametrów.

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `any` | `int` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `transform` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Zliczenie plików projektu:

```nurt example
find "/home/user/project" |> count
```

Zliczenie wierszy pustego pliku daje 0:

```nurt example
lines "/home/user/empty.txt" |> count
```

Zliczenie wyników uruchomienia programu:

```nurt example
run ["git", "status", "--short"] cwd: "/home/user/project" |> count
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_TYPE` | wejście jest pojedynczą wartością, a nie rekordami | zastosuj `count` do etapu, który tworzy rekordy |

## Powiązane {#related}

`assert`, `summarize`, `group`.
