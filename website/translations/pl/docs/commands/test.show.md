---
command: test.show
group: verify
summary: Pokazuje wyniki wcześniejszego przebiegu testów.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# test.show

## Składnia {#synopsis}

```text
test.show [<run_id>] [name: value]...
```

## Opis {#description}

Odczytuje zapisany przebieg i zwraca jego nagłówek `test_run` oraz rekordy
`test_result` pasujące do filtrów. Bez `status` zwracany jest każdy wynik.
`failed` wybiera wyniki ze statusem `failed` i `error`; `skipped` wybiera
wyniki ze statusem `skipped` i `todo`.

`test` zachowuje wyniki, których identyfikator lub nazwa zawiera dany tekst,
bez rozróżniania wielkości liter. `failures` ogranicza, ile wyników
zakończonych niepowodzeniem zachowuje komunikat, ślad stosu i wyjście, a
`message_lines` przycina każdy z tych tekstów do tej liczby wierszy.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `run_id` | `text` | najnowszy przebieg | Wcześniejszy przebieg testów. |
| `test` | `text` | brak | Tylko test o tym identyfikatorze lub nazwie. |
| `status` | `enum(failed\|passed\|skipped)` | brak | Tylko testy o tym statusie. |
| `failures` | `int` | `10` | Maksymalna liczba testów zakończonych niepowodzeniem zgłaszanych ze szczegółami. |
| `message_lines` | `int` | `5` | Maksymalna liczba wierszy komunikatu na test zakończony niepowodzeniem. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `none` | `test_result` |

Polecenie nie przyjmuje wejścia. Daje nagłówek `test_run` przebiegu i jego
rekordy `test_result` w kolejności przebiegu.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Pokazanie niepowodzeń najnowszego przebiegu, po trzy wiersze komunikatu.

```nurt example
test.show status: failed message_lines: 3
```

Pokazanie wyników jednego przebiegu, którego identyfikator zawiera nazwę.

```nurt example
test.show "run-2" test: "parse"
```

Pokazanie tylko testów zakończonych powodzeniem w najnowszym przebiegu.

```nurt example
test.show status: passed
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_UNKNOWN_RUN` | Przebieg nie istnieje; błąd wylicza znane przebiegi. | Użyj jednego z wymienionych przebiegów lub uruchom `test.run`. |

## Powiązane {#related}

`test.run`, `test.failed`, `test.list`.
