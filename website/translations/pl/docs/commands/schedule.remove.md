---
command: schedule.remove
group: schedule
summary: Usuwa harmonogramy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# schedule.remove

## Składnia {#synopsis}

```text
schedule.remove <id>... [name: value]...
```

## Opis {#description}

`schedule.remove` usuwa definicje wybranych harmonogramów w jednej operacji dziennika i zwraca jeden rekord `effect` dla każdego harmonogramu. Harmonogramy wybiera `id` albo `all: true`; każdy identyfikator musi istnieć.

Usunięcie harmonogramu zatrzymuje jego trwające uruchomienia: programy akcji otrzymują `terminate`, a po `run.kill_grace` sygnał `kill`. Uruchomienie oczekujące w kolejce `overlap: queue` jest odrzucane. Zatrzymane uruchomienia są wyświetlane w `after.stopped_jobs` rekordu skutku, a skutek jest oznaczany jako nieodwracalny przez `process_stopped`, ponieważ zatrzymanego programu nie można wznowić.

Historia uruchomień usuniętego harmonogramu pozostaje w stanie uruchomień i nadal jest widoczna w `schedule.history`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `id` | `list<text>` | none | Harmonogramy do usunięcia. |
| `all` | `bool` | `false` | Usuwa wszystkie harmonogramy. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none` | `effect` |

Rekord skutku ma `kind: schedule.remove`, `target: schedule:<id>`, `action: remove`, `before` ze stanem sprzed usunięcia i `after` ze `state: removed` oraz, gdy uruchomienia zostały zatrzymane, `stopped_jobs`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

Po cofnięciu (`undo`) przywrócony harmonogram liczy swój następny termin uruchomienia od chwili przywrócenia, więc terminy uruchomienia, które upłynęły w czasie usunięcia, nie są nadrabiane.

## Przykłady {#examples}

Usunięcie jednego harmonogramu:

```nurt example
schedule.remove "s1"
```

Usunięcie dwóch harmonogramów w jednej operacji:

```nurt example
schedule.remove ["s1", "s2"]
```

Usunięcie każdego harmonogramu:

```nurt example
schedule.remove all: true
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Nie podano ani `id`, ani `all: true`. | Podaj identyfikatory lub `all: true`. |
| `E_PARAM_CONFLICT` | `id` i `all: true` podano razem. | Zachowaj jedno z nich. |
| `E_NOT_FOUND` | Niektóre identyfikatory nie istnieją; `actual` je wyświetla. Nic nie zostało usunięte. | Popraw identyfikatory; `schedule.list` pokazuje istniejące. |
| `E_INVALID_VALUE` | Identyfikator zawiera znaki inne niż litery, cyfry, `-` i `_`. | Podaj identyfikatory takie jak `s3`. |
| `E_IO` | Magazynu harmonogramów nie można zapisać. | Sprawdź uprawnienia katalogu stanu. |

## Powiązane {#related}

`schedule.pause` wstrzymuje harmonogram bez jego usuwania. `schedule.list` pokazuje harmonogramy, a `schedule.history` ich minione uruchomienia.
