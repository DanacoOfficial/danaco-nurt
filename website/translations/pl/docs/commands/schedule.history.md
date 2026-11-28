---
command: schedule.history
group: schedule
summary: Wyświetla minione uruchomienia harmonogramów wraz z ich wynikiem i operacją dziennika.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# schedule.history

## Składnia {#synopsis}

```text
schedule.history [<id>] [name: value]...
```

## Opis {#description}

`schedule.history` zwraca uruchomienia zapisane przez usługę w tle, od najnowszego startu, w liczbie najwyżej `limit` rekordów. Z `id` zwraca tylko uruchomienia tego harmonogramu; uruchomienia usuniętego harmonogramu pozostają dostępne.

Każdy rekord opisuje jedno uruchomienie lub jeden termin uruchomienia, który nie został wykonany. `exit_state` zawiera wynik: `ok`, `failed` (kod zakończenia różny od zera, odrzucone lub nieudane uruchomienie), `timed_out`, `cancelled` (zatrzymane uruchomienie), `missed` (termin uruchomienia pominięty, ponieważ był spóźniony lub żaden klient nie dostarczył akcji) i `skipped_overlap` (termin uruchomienia osiągnięty, gdy trwało uruchomienie). `trigger_time` to termin uruchomienia lub start dla uruchomień ręcznych. `message` nazywa uruchomienie, jego wyzwalacz i wynik, na przykład `#4 schedule ok` lub `#5 schedule skipped (misfire)`. `operation` to operacja dziennika uruchomienia.

Usługa przechowuje `schedule.history_keep` uruchomień na harmonogram (wartość domyślna 20). Uruchomienia ręczne wykonane bez usługi nie są zapisywane.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `id` | `text` | none | Tylko uruchomienia tego harmonogramu. |
| `limit` | `int` | `20` | Maksymalna liczba uruchomień, od najnowszych. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none` | `schedule_run` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie ostatnich dwudziestu uruchomień wszystkich harmonogramów:

```nurt example
schedule.history
```

Wyświetlenie ostatnich pięciu uruchomień jednego harmonogramu:

```nurt example
schedule.history "s1" limit: 5
```

Wyświetlenie uruchomień, które nie zakończyły się powodzeniem:

```nurt example
schedule.history limit: 100 |> filter exit_state != "ok" |> select schedule_id started message
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | Żaden harmonogram ani zapisane uruchomienie nie ma identyfikatora `id`. | Wyświetl harmonogramy poleceniem `schedule.list`. |
| `E_INVALID_VALUE` | `limit` jest mniejsze niż 1 lub `id` nie jest identyfikatorem harmonogramu. | Podaj dodatnie `limit` i identyfikator taki jak `s3`. |
| `E_FORMAT` | `<state>/schedules/state.json` jest uszkodzony. | Uruchom ponownie usługę w tle; zapisze plik na nowo. |
| `E_IO` | Magazynu harmonogramów nie można odczytać. | Sprawdź uprawnienia katalogu stanu. |

## Powiązane {#related}

`schedule.list` wyświetla ostatni wynik każdego harmonogramu; `schedule.run` uruchamia harmonogram poza jego terminami uruchomienia.
