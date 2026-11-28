---
command: schedule.list
group: schedule
summary: Wyświetla harmonogramy wraz z ich następnym uruchomieniem i stanem.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# schedule.list

## Składnia {#synopsis}

```text
schedule.list [<id>] [name: value]...
```

## Opis {#description}

`schedule.list` zwraca jeden rekord `schedule` dla każdego harmonogramu, uporządkowany według identyfikatora. Bez parametrów wyświetla każdy harmonogram konta; `state` zachowuje harmonogramy w jednym stanie, a `id` zwraca jeden harmonogram.

Stanem harmonogramu jest `paused`, gdy jest wstrzymany, `completed` po `max_runs` uruchomieniach rozpoczętych w terminach uruchomienia, `expired` po upływie jego czasu wygaśnięcia lub po pominiętym pojedynczym czasie, a w pozostałych przypadkach `active`. `active` ma wartość `true` tylko dla aktywnego harmonogramu, gdy działa usługa w tle; w przeciwnym razie `reason` podaje przyczynę: `service_not_running`, `paused`, `completed` lub `expired`.

`next_run`, `last_run`, `runs` (uruchomienia rozpoczęte w terminach uruchomienia) i `last_result` pochodzą ze stanu uruchomień przechowywanego przez usługę w tle. Harmonogram, którego usługa jeszcze nie widziała, pokazuje następny termin uruchomienia obliczony z jego wyzwalacza. Czasy są chwilami; postać tekstowa pokazuje je z przesunięciem strefy czasowej harmonogramu.

Polecenie odczytuje magazyn harmonogramów i niczego nie zmienia.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `id` | `text` | none | Harmonogram do pokazania wraz z jego ostatnimi pięcioma uruchomieniami. |
| `state` | `enum(all\|active\|paused\|completed\|expired)` | `all` | Tylko harmonogramy w tym stanie. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none` | `schedule` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie wszystkich harmonogramów:

```nurt example
schedule.list
```

Wyświetlenie wstrzymanych harmonogramów:

```nurt example
schedule.list state: paused
```

Pokazanie jednego harmonogramu:

```nurt example
schedule.list "s3"
```

Wyświetlenie etykiet i następnych terminów uruchomienia aktywnych harmonogramów, które nie działają:

```nurt example
schedule.list state: active |> filter not active |> select label next_run reason
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | Żaden harmonogram nie ma identyfikatora `id`. `actual` wyświetla identyfikator. | Wyświetl harmonogramy poleceniem `schedule.list` i użyj jednego z ich identyfikatorów. |
| `E_INVALID_VALUE` | `id` nie jest identyfikatorem harmonogramu lub `state` jest nieznany. | Podaj identyfikator taki jak `s3` i jeden z wymienionych stanów. |
| `E_FORMAT` | `<state>/schedules/state.json` jest uszkodzony. | Uruchom ponownie usługę w tle; zapisze plik na nowo. |
| `E_IO` | Magazynu harmonogramów nie można odczytać. | Sprawdź uprawnienia katalogu stanu. |

## Powiązane {#related}

`schedule.history` wyświetla minione uruchomienia harmonogramów; `schedule.create`, `schedule.pause`, `schedule.resume` i `schedule.remove` je zmieniają.
