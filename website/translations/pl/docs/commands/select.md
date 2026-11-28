---
command: select
group: core
summary: Zachowuje podane pola każdego rekordu i dodaje pola obliczane.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# select

## Składnia {#synopsis}

```text
any |> select <fields>...
```

## Opis {#description}

`select` zachowuje wskazane pola każdego rekordu, w zapisanej kolejności, i dodaje pola obliczane zapisane jako `name: expression`. Ścieżka pola, taka jak `owner.name`, odczytuje pole wewnątrz innego pola i jest nazwana według swojego ostatniego segmentu.

Rekordy zachowujące tylko pola najwyższego poziomu zachowują swój rodzaj rekordu. Rekordy z polem obliczanym lub z polem odczytanym z wnętrza innego pola stają się rekordami `map`. Pole, którego rekord nie ma, zatrzymuje program błędem `E_UNKNOWN_FIELD`, który wyświetla pola rekordu.

Etap jest leniwy i zachowuje nagłówek strumienia. Kolejność zadeklarowana przez źródło i jego wskazówka prezentacji są odrzucane, ponieważ mogą wskazywać pola, których już nie ma. Puste wejście daje puste wyjście.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `fields` | `list<field \| named_expression>` | wymagany | Pola do zachowania oraz pary nazwa: wyrażenie do obliczenia. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `any` | `any` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `transform` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Zachowanie ścieżki i rozmiaru każdego pliku:

```nurt example
find "/home/user/project" |> select path size
```

Dodanie pola obliczanego oznaczającego duże pliki:

```nurt example
find "/home/user/project" |> select path large: size > 2kb
```

Zachowanie kodu zakończenia i wyjścia uruchomionego programu:

```nurt example
run ["git", "status", "--short"] cwd: "/home/user/project" |> select exit_code stdout
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | nie podano żadnego pola | podaj co najmniej jedno pole |
| `E_UNKNOWN_FIELD` | pola nie ma w rekordzie; `expected` wyświetla pola | użyj jednego z wymienionych pól |
| `E_TYPE` | element nie jest ani polem, ani `name: expression` | zapisz nazwę pola lub `name: expression` |

## Powiązane {#related}

`filter`, `sort`, `distinct`, `describe`.
