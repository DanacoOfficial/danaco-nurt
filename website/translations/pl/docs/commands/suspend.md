---
command: suspend
group: system
summary: Wstrzymuje zadania lub procesy wejściowe do czasu ich wznowienia.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# suspend

## Składnia {#synopsis}

```text
job|process |> suspend
```

## Opis {#description}

Wstrzymuje całe drzewo procesów każdego zadania wejściowego: `SIGSTOP` do
grupy procesów w systemach Linux i macOS, wstrzymanie wątków obiektu zadania
w systemie Windows. Stan zadania zmienia się na `suspended`. Czas spędzony
we wstrzymaniu nie jest wliczany do limitu czasu zadania i jest zgłaszany w
`suspended_total` rekordu końcowego.

Wstrzymać można tylko uruchomione zadania. Rekordy zadań i procesów obsługuje
ta implementacja.

## Parametry {#parameters}

Polecenie nie ma parametrów.

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `job`, `process` | `effect` |

Wejściem są rekordy `job` lub `process`. Daje jeden rekord `effect` na każdy
rekord wejściowy; jego pole `job` zawiera rekord zadania po zmianie.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `run` |
| Skutek | `external` |
| Odwracalność | `irreversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wstrzymanie zadania `j7`.

```nurt example
jobs "j7" |> suspend
```

```text expected
suspended  job:j7
```

Wstrzymanie wszystkich uruchomionych zadań.

```nurt example
jobs state: running |> suspend
```

Wstrzymanie zadania `j3` i odczyt jego stanu.

```nurt example
jobs "j3" |> suspend |> select job.state
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_JOB_STATE` | Zadanie nie jest uruchomione, na przykład jest już wstrzymane. | Wznów je: `jobs "j7" \|> resume`. |
| `E_UNKNOWN_JOB` | Żadne zadanie nie ma tego identyfikatora. | Użyj jednego z identyfikatorów w `candidates`. |

## Powiązane {#related}

`resume`, `stop`, `jobs`.
