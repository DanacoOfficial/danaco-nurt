---
command: schedule.run
group: schedule
summary: Uruchamia akcję harmonogramu teraz, poza jego terminami i limitem uruchomień.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# schedule.run

## Składnia {#synopsis}

```text
schedule.run <id>
```

## Opis {#description}

`schedule.run` uruchamia akcję harmonogramu jednokrotnie i zwraca rekordy wyjściowe akcji, na przykład rekord `exec` etapu `run`. Uruchomienie nie zmienia następnego terminu uruchomienia i nie wlicza się do `max_runs`. Można uruchamiać także harmonogramy wstrzymane, zakończone i wygasłe.

Przy działającej usłudze w tle usługa wykonuje uruchomienie jako operację sesji `schedule:<id>`, zapisuje je w historii z `trigger: manual` i wraca po jego zakończeniu. Trwające uruchomienie tego samego harmonogramu, zaplanowane lub ręczne, powoduje niepowodzenie polecenia z błędem `E_JOB_STATE`; blokadę współdzielą usługa i każdy proces konsoli.

Bez usługi proces wywołujący sam wykonuje akcję rodzaju `pipeline`, w bieżącej sesji; takie uruchomienie nie jest zapisywane w historii. Akcje innych rodzajów działają tylko w usłudze.

Definicja, której akcja nie zgadza się już ze skrótem integralności, nie jest uruchamiana.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `id` | `text` | wymagany | Harmonogram do uruchomienia. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none` | `exec`, `job` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `run` |
| Skutek | `process` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `builtin` |
| Warunki odwracalności | Zmiany plików są odwracalne jak dla run; skutki poza systemem plików są nieodwracalne. |

## Przykłady {#examples}

Uruchomienie harmonogramu teraz:

```nurt example
schedule.run "s1"
```

Uruchomienie harmonogramu i zachowanie jego kodu zakończenia oraz standardowego wyjścia:

```nurt example
schedule.run "s1" |> select exit_code stdout
```

Utworzenie harmonogramu i natychmiastowe sprawdzenie jego akcji:

```nurt example
schedule.create "report" cron: "0 7 * * mon" action: (run ["./weekly-report.sh"] cwd: "/srv/reports")
schedule.run "s1"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Brakuje `id`. | Podaj identyfikator harmonogramu. |
| `E_NOT_FOUND` | Żaden harmonogram nie ma identyfikatora `id`. | Wyświetl harmonogramy poleceniem `schedule.list`. |
| `E_JOB_STATE` | Uruchomienie harmonogramu trwa; `actual` zawiera jego identyfikator, na przykład `s1#4`. | Poczekaj na zakończenie uruchomienia lub śledź je poleceniem `schedule.history`. |
| `E_STAGE_FAILED` | Akcja nie może zostać tu uruchomiona: jej rodzaj działa tylko w usłudze albo jej definicja została zmieniona poza poleceniami harmonogramu. | Uruchom usługę w tle albo usuń harmonogram i utwórz go ponownie. |
| `E_INVALID_VALUE` | `id` nie jest identyfikatorem harmonogramu. | Podaj identyfikator taki jak `s3`. |

Błędy samej akcji, takie jak `E_TIMEOUT`, są zwracane tak, jak zgłaszają je jej etapy.

## Powiązane {#related}

`schedule.history` wyświetla uruchomienia wykonane przez usługę; `schedule.create` z `run_now: true` uruchamia nowy harmonogram przy tworzeniu.
