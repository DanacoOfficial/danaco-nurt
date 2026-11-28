---
command: stop
group: core
summary: Zatrzymuje zadania, procesy lub usługi.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# stop

## Składnia {#synopsis}

```text
job|process|service |> stop [name: value]...
```

## Opis {#description}

Zatrzymuje każde zadanie wejściowe: sygnał `signal` trafia do całej grupy
procesów lub obiektu zadania programu, a po upływie `grace` pozostałe procesy
są zabijane. Polecenie wraca po zakończeniu zadania i zapisaniu jego rekordu
końcowego, wykrytych zmian oraz wpisu dziennika.

Zatrzymać można tylko zadania uruchomione i wstrzymane. Rekordy zadań i
procesów obsługuje ta implementacja (sygnał trafia do procesu rekordu
`process`); rekordy usług obsługują implementacje poleceń systemowych,
wybierane według rodzaju rekordu wejściowego.

W systemie Windows `terminate`, `interrupt` i `hangup` żądają zakończenia
grupy programów konsoli; `kill` kończy obiekt zadania.

**Uprawnienie.** Jednostki systemowe wymagają administratora: zapisz `privilege: elevated` na etapie. `undo` wpisu uruchamia usługi, które zatrzymało polecenie; zatrzymanie procesu lub zadania jest nieodwracalne.

```nurt
services name: "nginx" |> stop privilege: elevated
```

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `signal` | `enum(terminate\|kill\|interrupt\|hangup)` | `terminate` | Sygnał wysyłany jako pierwszy. |
| `grace` | `duration` | `5s` | Czas oczekiwania, zanim proces zostanie zabity. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `job`, `process`, `service` | `effect` |

Wejściem są rekordy `job`, `process` lub `service`. Daje jeden rekord `effect`
na każdy rekord wejściowy; dla zadania jego pole `job` zawiera końcowy rekord
zadania ze stanem `stopped`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `run` |
| Skutek | `external` |
| Odwracalność | `irreversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Zatrzymanie zadania `j7`.

```nurt example
jobs "j7" |> stop
```

```text expected
stopped  job:j7
```

Zatrzymanie od razu wszystkich uruchomionych zadań, bez okresu karencji.

```nurt example
jobs state: running |> stop signal: kill
```

Przerwanie zadania `j3` i zabicie go po dziesięciu sekundach.

```nurt example
jobs "j3" |> stop signal: interrupt grace: 10s
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_JOB_STATE` | Zadanie już się zakończyło albo utraciło nadzorcę. | Odczytaj zadanie za pomocą `jobs "j7"`. |
| `E_UNKNOWN_JOB` | Żadne zadanie nie ma tego identyfikatora. | Użyj jednego z identyfikatorów w `candidates`. |
| `E_TIMEOUT` | Nadzorca zadania nie odpowiedział. | Odczytaj stan za pomocą `jobs "j7"`. |

## Powiązane {#related}

`jobs`, `suspend`, `resume`, `remove`.
