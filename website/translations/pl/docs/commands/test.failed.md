---
command: test.failed
group: verify
summary: Uruchamia ponownie tylko testy, które zawiodły we wcześniejszym przebiegu.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# test.failed

## Składnia {#synopsis}

```text
test.failed [<run_id>] [name: value]...
```

## Opis {#description}

Wybiera testy ze statusem `failed` lub `error` z wcześniejszego przebiegu i
uruchamia tylko je. Identyfikatory testów są tłumaczone na selektory
frameworka: pliki i wzorzec nazwy dla bun, vitest i jest, identyfikatory
węzłów dla pytest, pakiety i `-run` dla go oraz nazwy `--exact` dla cargo.
Skrypty `npm` i polecenia własne nie mogą wybierać pojedynczych testów;
zapisane polecenie jest uruchamiane ponownie, a polecenie zapisuje notatkę.

Bez `run_id` przebiegiem jest najnowszy przebieg, który ma testy zakończone
niepowodzeniem i którego żaden późniejszy przebieg nie powtórzył. Przy `path`
brane są pod uwagę tylko przebiegi z tego katalogu projektu i w nim
wykonywane jest powtórzenie. Nowy przebieg zapisuje powtarzany przebieg w
`retry_of`.

Gdy przebieg nie ma testów zakończonych niepowodzeniem, polecenie zwraca swój
nagłówek `test_run` i brak wyników, i niczego nie uruchamia.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `run_id` | `text` | najnowszy przebieg | Wcześniejszy przebieg testów. |
| `path` | `path` | brak | Katalog projektu wcześniejszego przebiegu. |
| `timeout` | `duration` | `30m` | Limit czasu przebiegu testów. |
| `wait` | `duration` | `3s` | Czas oczekiwania, zanim przebieg będzie kontynuowany w tle jako zadanie. |
| `failures` | `int` | `10` | Maksymalna liczba testów zakończonych niepowodzeniem zgłaszanych ze szczegółami. |
| `message_lines` | `int` | `5` | Maksymalna liczba wierszy komunikatu na test zakończony niepowodzeniem. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `none` | `test_run`, `test_result`, `job` |

Polecenie nie przyjmuje wejścia. Wyjście jest takie samo jak wyjście
`test.run`: nagłówek `test_run`, po którym następują rekordy `test_result`,
albo rekord `job`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `run` |
| Skutek | `process` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `user_program` |
| Warunki odwracalności | Pliki zmienione przez programy testowe są odwracalne, gdy ich poprzednia zawartość została zachowana; skutki poza systemem plików są nieodwracalne. |

## Przykłady {#examples}

Ponowne uruchomienie testów, które zawiodły w najnowszym przebiegu.

```nurt example
test.failed
```

Ponowne uruchomienie testów, które zawiodły w wskazanym przebiegu, i pokazanie nieudanych wyników.

```nurt example
test.failed "run-3" |> filter status == "failed"
```

Ponowne uruchomienie testów, które zawiodły w przebiegu z innego katalogu projektu.

```nurt example
test.failed path: "/work/service"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_UNKNOWN_RUN` | Przebieg nie istnieje albo żaden przebieg nie ma testów zakończonych niepowodzeniem. | Wylistuj przebiegi za pomocą `test.list`. |
| `E_EXTERNAL_MISSING` | Program testowy nie jest zainstalowany. | Zainstaluj go. |
| `E_OUTPUT_PARSE` | Nie można odczytać raportu ani wyjścia. | Uruchom ponownie za pomocą `test.run command:`. |

## Powiązane {#related}

`test.run`, `test.show`, `test.list`.
