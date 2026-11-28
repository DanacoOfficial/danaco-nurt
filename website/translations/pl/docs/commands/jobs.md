---
command: jobs
group: core
summary: Odczytuje stan, wyjście lub wynik zadań w tle.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# jobs

## Składnia {#synopsis}

```text
jobs [<id>] [name: value]...
```

## Opis {#description}

Każdy program uruchomiony przez `run`, `retry`, `test.*` i `analyze.*` jest
zadaniem od chwili uruchomienia i ma identyfikator `j1`, `j2`, …, który nigdy
nie jest ponownie używany w katalogu stanu. Program, który nadal działa po
`wait` swojego polecenia, kontynuuje w tle; `jobs` odczytuje jego stan, wyjście
i wynik końcowy.

Bez `id` polecenie wyświetla wszystkie zadania: najpierw zadania uruchomione i
zawieszone, potem zakończone, każda grupa w kolejności identyfikatorów. `state`
wybiera zadania w jednym stanie.

Z `id` polecenie zwraca jeden rekord zadania, wypełniony zgodnie z `view`:

* `status`: stan, czas, który upłynął, i ostatnie 5 wierszy każdego strumienia
  wyjścia;
* `output`: kompletne wiersze zapisane po kursorze `since` albo całe wyjście
  bez `since`; `output_cursor` wyniku jest kursorem do następnego odczytu.
  `stream`, `head`, `tail`, `filter`, `context` i `dedupe` wybierają wiersze;
  kursor strumienia pominiętego przez `stream` pozostaje tam, gdzie był, więc
  późniejszy odczyt tego strumienia nie pomija żadnego wiersza;
* `result`: końcowy rekord zakończonego zadania (`exit_code`, `signal`,
  `elapsed`, `changes`, `op`, pliki wyjścia) z zachowanym wyjściem wybranym tymi
  samymi parametrami. Dla zadania `test.*` lub `analyze.*` nagłówek strumienia
  zawiera końcowy rekord narzędzia, czyli `test_run` lub `analysis`.

`wait` blokuje wywołanie do zakończenia zadania albo upływu czasu, zależnie od
tego, co nastąpi wcześniej; rekord zadania jest wtedy zwracany w stanie, w jakim
jest.

Zadanie, którego proces nadzorujący zakończył się bez jego ukończenia, ma stan
`lost`: jego program może nadal działać, ale nic nie zapisuje jego wyjścia ani
zmian. Stan jest ustalany przy każdym odczycie: każde nadzorowane zadanie
trzyma blokadę, którą system operacyjny zwalnia, gdy proces nadzorujący się
kończy, niezależnie od sposobu zakończenia.

Zakończone zadania, których koniec jest starszy niż `output.spool_ttl`, są
usuwane wraz z wyjściem przy wyświetlaniu zadań i przy uruchomieniu
nadzorcy. Zadania uruchomione nigdy nie są usuwane.

`remove` na rekordach `job` usuwa zakończone zadania: wpis każdego zadania w
tablicy zadań i jego zachowane wyjście w katalogu stanu. Wpisy zadań i ich
wyjście są wewnętrznym stanem konsoli, a nie plikami użytkownika, więc usunięcie
nie dodaje wpisu dziennika, nie może zostać cofnięte i nie jest oznaczane jako
nieodwracalne. Można usunąć zadania w stanach `exited`, `failed`, `stopped`,
`timed_out`, `stalled`, `limit` i `lost`; dla zadania `lost` usuwane są tylko
wpis i wyjście, a program, który nadal działa, nie otrzymuje sygnału. Zadanie w
stanie `running` lub `suspended` jest odrzucane z `E_JOB_STATE`; `stop` kończy je
najpierw. Wejście jest sprawdzane w całości przed usunięciem pierwszego zadania.
Każde usunięte zadanie daje jeden rekord `effect` z `kind` i `action` równymi
`remove`, `target` równym `job:<id>` oraz ostatnim rekordem `job`. Usunięty
identyfikator nie jest używany ponownie, a późniejsze odwołanie do niego kończy
się błędem `E_UNKNOWN_JOB`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `id` | `text` | brak | Identyfikator zadania, taki jak j7. |
| `state` | `enum(any\|running\|suspended\|exited\|failed\|stopped\|timed_out\|stalled\|limit\|lost)` | `any` | Stan zadań do wyświetlenia. |
| `head` | `int` | brak | Liczba zwracanych pierwszych wierszy wyjścia. |
| `tail` | `int` | brak | Liczba zwracanych ostatnich wierszy wyjścia. |
| `stream` | `enum(both\|stdout\|stderr)` | `both` | Strumień wyjścia do odczytu. |
| `since` | `text` | brak | Kursor wcześniejszego odczytu; zwracane jest tylko nowsze wyjście. |
| `wait` | `duration` | `0s` | Czas oczekiwania na zakończenie zadania. |
| `view` | `enum(status\|output\|result)` | `status` | Stan, wyjście lub wynik końcowy zadania. |
| `filter` | `regex` | brak | Zwraca tylko pasujące wiersze wyjścia. |
| `context` | `int` | `0` | Wiersze kontekstu wokół wierszy pasujących do filter. |
| `dedupe` | `bool` | `false` | Zwija powtarzające się identyczne wiersze wyjścia. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `job` |

Tylko źródło. Tworzy rekordy `job`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetl uruchomione zadania.

```nurt example
jobs state: running
```

```text expected
j7  running  cargo build
```

Odczytaj wyjście zadania `j7` zapisane po wcześniejszym odczycie.

```nurt example
jobs "j7" view: output since: "120.4"
```

Poczekaj do jednej minuty na zadanie `j7` i zwróć jego rekord końcowy.

```nurt example
jobs "j7" view: result wait: 60s
```

Usuń wszystkie zadania, które zakończyły się kodem 0, wraz z ich wyjściem.

```nurt example
jobs state: exited |> remove
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_UNKNOWN_JOB` | Żadne zadanie nie ma tego identyfikatora. | Użyj jednego z identyfikatorów w `candidates`. |
| `E_JOB_STATE` | `view: result` dla zadania, które nadal działa. | Dodaj `wait:` albo odczytaj `view: status`. |
| `E_JOB_STATE` | `remove` dla zadania uruchomionego lub zawieszonego. | Zakończ zadanie poleceniem `stop`, a następnie usuń je. |
| `E_CONFIG` | Silnik nie ma katalogu stanu. | Skonfiguruj katalog stanu silnika. |

## Powiązane {#related}

`run`, `retry`, `stop`, `suspend`, `resume`, `remove`.
