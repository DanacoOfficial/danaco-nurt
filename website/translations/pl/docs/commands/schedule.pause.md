---
command: schedule.pause
group: schedule
summary: Wstrzymuje harmonogramy bez ich usuwania.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# schedule.pause

## Składnia {#synopsis}

```text
schedule.pause <id>... [name: value]...
```

## Opis {#description}

`schedule.pause` oznacza wybrane aktywne harmonogramy jako wstrzymane w jednej operacji dziennika i zwraca jeden rekord `effect` dla każdego wybranego harmonogramu. Wstrzymany harmonogram zachowuje swoją definicję i historię i nie wykonuje uruchomień; terminy uruchomienia, które upłynęły w czasie wstrzymania, nie są nadrabiane. Trwające uruchomienie nie jest zatrzymywane.

Zmieniają się tylko harmonogramy w stanie `active`. Harmonogramy wstrzymane, zakończone i wygasłe są zgłaszane z `action: unchanged` i swoim stanem w `before`, a dla nich nie jest zapisywany żaden plik.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `id` | `list<text>` | none | Harmonogramy do wstrzymania. |
| `all` | `bool` | `false` | Wstrzymuje wszystkie harmonogramy. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none` | `effect` |

Rekord skutku ma `kind: schedule.pause`, `target: schedule:<id>`, `action` (`pause` lub `unchanged`), `before` z poprzednim stanem i `after` z nowym stanem.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wstrzymanie jednego harmonogramu:

```nurt example
schedule.pause "s2"
```

Wstrzymanie dwóch harmonogramów w jednej operacji:

```nurt example
schedule.pause ["s2", "s4"]
```

Wstrzymanie każdego harmonogramu, na przykład przed konserwacją urządzenia:

```nurt example
schedule.pause all: true
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Nie podano ani `id`, ani `all: true`. | Podaj identyfikatory lub `all: true`. |
| `E_PARAM_CONFLICT` | `id` i `all: true` podano razem. | Zachowaj jedno z nich. |
| `E_NOT_FOUND` | Niektóre identyfikatory nie istnieją; `actual` je wyświetla. Nic nie zostało zmienione. | Popraw identyfikatory; `schedule.list` pokazuje istniejące. |
| `E_INVALID_VALUE` | Identyfikator zawiera znaki inne niż litery, cyfry, `-` i `_`. | Podaj identyfikatory takie jak `s3`. |
| `E_IO` | Magazynu harmonogramów nie można zapisać. | Sprawdź uprawnienia katalogu stanu. |

## Powiązane {#related}

`schedule.resume` wznawia wstrzymane harmonogramy; `schedule.list state: paused` je wyświetla.
