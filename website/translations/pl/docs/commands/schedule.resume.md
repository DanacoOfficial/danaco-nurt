---
command: schedule.resume
group: schedule
summary: Wznawia wstrzymane harmonogramy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# schedule.resume

## Składnia {#synopsis}

```text
schedule.resume <id>... [name: value]...
```

## Opis {#description}

`schedule.resume` ponownie czyni wybrane wstrzymane harmonogramy aktywnymi w jednej operacji dziennika i zwraca jeden rekord `effect` dla każdego wybranego harmonogramu. Następny termin uruchomienia jest liczony od chwili wznowienia: harmonogram z `every: 10m` wznowiony o 15:07 uruchomi się następnie o 15:17. Uruchomienie, które przed wstrzymaniem oczekiwało w kolejce `overlap: queue`, jest odrzucane. Pojedynczy czas (`at`), który upłynął w czasie wstrzymania, jest obsługiwany przez `on_missed`.

Zmieniają się tylko harmonogramy w stanie `paused`. Harmonogramy aktywne, zakończone i wygasłe są zgłaszane z `action: unchanged` i swoim stanem w `before`, a dla nich nie jest zapisywany żaden plik.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `id` | `list<text>` | none | Harmonogramy do wznowienia. |
| `all` | `bool` | `false` | Wznawia wszystkie wstrzymane harmonogramy. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none` | `effect` |

Rekord skutku ma `kind: schedule.resume`, `target: schedule:<id>`, `action` (`resume` lub `unchanged`), `before` z poprzednim stanem i `after` z nowym stanem.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wznowienie jednego harmonogramu:

```nurt example
schedule.resume "s2"
```

Wznowienie dwóch harmonogramów w jednej operacji:

```nurt example
schedule.resume ["s2", "s4"]
```

Wznowienie każdego wstrzymanego harmonogramu i wyświetlenie następnych terminów uruchomienia:

```nurt example
schedule.resume all: true
schedule.list state: active |> select id label next_run
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

`schedule.pause` wstrzymuje harmonogramy; `schedule.list` pokazuje ich stan i następny termin uruchomienia.
