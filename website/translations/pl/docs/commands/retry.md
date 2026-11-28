---
command: retry
group: core
summary: Uruchamia ponownie polecenie zadania z tym samym katalogiem roboczym i środowiskiem.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# retry

## Składnia {#synopsis}

```text
job |> retry [name: value]...
```

## Opis {#description}

Uruchamia ponownie polecenie każdego zadania z wejścia według reguł `run`: ten sam program i argumenty, katalog roboczy, środowisko, standardowe wejście, limity, wykrywanie zmian i wybór wyjścia. Nowe zadanie zapisuje powtarzane zadanie w `retry_of`. Zadanie `test.*` lub `analyze.*` jest powtarzane jako takie, tak że jego rekord końcowy jest tworzony ponownie.

Środowisko zadania jest przechowywane poza rekordem zadania, w pliku katalogu stanu czytelnym tylko dla konta silnika. Ani rekordy zadań, ani zdarzenia nie zawierają wartości środowiska.

Powtórzyć można tylko zakończone zadania. Program, który konsola kończy z powodu limitu lub anulowania, powoduje niepowodzenie wywołania jak w `run`; gdy powtarzanych jest kilka zadań, błąd ma ustawione `effect_applied`, gdy którekolwiek z nich się rozpoczęło. Gdy nowy program działa dłużej niż `wait`, kontynuuje w tle; wynikiem jest wtedy rekord `job`, a etapy po `retry` są pomijane (`post_stages_deferred`).

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `timeout` | `duration` | limit czasu oryginalnego zadania | Limit czasu programu. |
| `wait` | `duration \| enum(forever)` | domyślne oczekiwanie run | Czas oczekiwania na program, zanim będzie kontynuowany jako zadanie. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `job` | `exec`, `job` |

Wejściem są rekordy `job`. Tworzy rekord `exec` dla każdego programu, który zakończył się w czasie `wait`, i rekord `job` dla każdego programu, który kontynuuje w tle.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `run` |
| Skutek | `process` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `user_program` |
| Warunki odwracalności | Jak dla run: zmiany obserwowanych plików są odwracalne, gdy zostały zachowane; pozostałe skutki są nieodwracalne. |

## Przykłady {#examples}

Powtórzenie zadania `j7`.

```nurt example
jobs "j7" |> retry
```

```text expected
exit_code: 0
```

Powtórzenie zadania `j7` z dłuższym limitem czasu i oczekiwanie na jego zakończenie.

```nurt example
jobs "j7" |> retry timeout: 30m wait: forever
```

Powtórzenie każdego nieudanego zadania w tle.

```nurt example
jobs state: failed |> retry wait: 0s
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_JOB_STATE` | Zadanie nadal działa. | Najpierw je zatrzymaj: `jobs "j7" \|> stop`. |
| `E_UNKNOWN_JOB` | Żadne zadanie nie ma tego identyfikatora. | Użyj jednego z identyfikatorów z `candidates`. |
| `E_PROGRAM_NOT_FOUND` | Programu nie można uruchomić ponownie. | Sprawdź program i katalog roboczy. |
| `E_SPAWN` | Programu nie można uruchomić ponownie. | Sprawdź program i katalog roboczy. |
| `E_IO` | Zapisane środowisko i standardowe wejście zadania są niedostępne lub nieczytelne. | Uruchom polecenie ponownie za pomocą `run`. |
| `E_TIMEOUT` | Program osiągnął `timeout` lub `idle_timeout`; `details.exec` zawiera rekord. | Podnieś limit lub powtórz z `wait: 0s`. |
| `E_LIMIT` | Program przekroczył `max_cpu` lub `max_memory`; `details.exec` zawiera rekord. | Uruchom polecenie ponownie z wyższym limitem. |
| `E_CANCELLED` | Wywołanie zostało anulowane w trakcie działania programu; `details.exec` zawiera rekord. | Powtórz wywołanie. |

## Powiązane {#related}

`run`, `jobs`, `stop`.
