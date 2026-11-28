---
command: resume
group: system
summary: Wznawia wstrzymane zadania lub procesy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# resume

## Składnia {#synopsis}

```text
job|process |> resume
```

## Opis {#description}

Wznawia drzewo procesów każdego zadania z wejścia wstrzymanego poleceniem `suspend`: `SIGCONT` dla grupy procesów w systemach Linux i macOS, wznowienie wątków obiektu zadania w systemie Windows. Stan zadania znów staje się `running`, a jego limit czasu biegnie dalej od miejsca, w którym się zatrzymał.

Wznowić można tylko wstrzymane zadania. Rekordy zadań i procesów są obsługiwane przez tę implementację.

## Parametry {#parameters}

Polecenie nie ma parametrów.

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `job`, `process` | `effect` |

Wejściem są rekordy `job` lub `process`. Tworzy jeden rekord `effect` dla każdego rekordu wejściowego; jego pole `job` zawiera rekord zadania po zmianie.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `run` |
| Skutek | `external` |
| Odwracalność | `irreversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wznowienie zadania `j7`.

```nurt example
jobs "j7" |> resume
```

```text expected
resumed  job:j7
```

Wznowienie wszystkich wstrzymanych zadań.

```nurt example
jobs |> filter state == "suspended" |> resume
```

Wznowienie zadania `j3` i oczekiwanie na jego zakończenie.

```nurt example
jobs "j3" |> resume
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_JOB_STATE` | Zadanie nie jest wstrzymane. | Najpierw je wstrzymaj: `jobs "j7" \|> suspend`. |
| `E_UNKNOWN_JOB` | Żadne zadanie nie ma tego identyfikatora. | Użyj jednego z identyfikatorów z `candidates`. |

## Powiązane {#related}

`suspend`, `stop`, `jobs`.
