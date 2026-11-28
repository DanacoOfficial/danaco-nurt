---
command: next
group: core
summary: Odczytuje następną część skróconego wyniku.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# next

## Składnia {#synopsis}

```text
next [<id>] [<count>] [name: value]...
```

## Opis {#description}

`next` zwraca następną część wyniku, który wcześniejsza odpowiedź skróciła.
Pierwsza odpowiedź polecenia pozostaje krótka; jedno kolejne wywołanie zwraca
następną część, potem następną, aż do końca. Odpowiedź, która pomija wiersze,
rekordy, bajty lub końce długich wierszy, zawiera pole `continuation`, a jej
tekst kończy się wskazówką „more: next”.

| Pole `continuation` | Znaczenie |
|---|---|
| `id` | krótki identyfikator, `c1`, `c2`, … w obrębie sesji konsoli |
| `unit` | `lines`, `records` lub `bytes` |
| `shown` | jednostki w dotychczas udzielonych odpowiedziach |
| `remaining` | jednostki jeszcze niezwrócone; `-1`, gdy nie są znane z góry |
| `cut_lines` | wiersze pokazanej części, które zostały skrócone |

Wskazówką jest „more: next”, gdy kontynuacja jest najnowsza, a w przeciwnym
razie „more: next c3”. Formy wywołania to:

| Wywołanie | Forma skrócona | Wynik |
|---|---|---|
| `next` | `next` | następna część najnowszej kontynuacji |
| `next id: c3` | `next c3` | następna część kontynuacji `c3` |
| `next count: 200` | `next 200` | część o 200 jednostkach; wartość domyślna to rozmiar pierwszej odpowiedzi |
| `next c3 count: 200` | `next c3 200` | oba |
| `next id: c3 all: true` | `next c3 all` | wszystko, co zostało z `c3` |
| `next id: c3 full_lines: true` | `next c3 full` | skrócone wiersze `c3`, w całości |
| `next all: true` | `next all` | wszystko, co zostało |
| `next full_lines: true` | `next full` | wiersze ostatnio pokazanej części, które zostały skrócone, w całości |

Identyfikator, liczba oraz słowa `all` i `full` są pozycyjne; samo słowo `all`
lub `full` oznacza parametr o tym samym znaczeniu. Formy długie pozostają
ważne i dają ten sam wynik.

Część ma ten sam typ rekordu co pierwsza odpowiedź polecenia odczytu oraz typ
rekordu `line` dla wyjścia `run`, więc potok może ją kontynuować:
`next |> filter severity == "error"`. `next` nigdy nie oblicza wyniku
ponownie. Odczytuje resztę ze źródła, które zachowała pierwsza odpowiedź:

| Polecenie | Źródło kontynuacji |
|---|---|
| `run` | plik wyjściowy uruchomienia (wiersze): wiersze między początkiem a końcem odpowiedzi; wybór wierszy jest kontynuowany od pierwszego wiersza pliku |
| `search`, `find`, `list`, `read`, `lines`, `diff`, `git.log`, `git.diff`, `git.blame`, `history`, `processes`, `ports`, `services`, `package.list`, `package.search`, `query`, `sql` i inne polecenia odczytu | obliczony wynik zachowany w katalogu stanu, w kolejności odpowiedzi |
| `logs` | kursor zapytania z chwili pierwszej odpowiedzi, tak aby części nie nakładały się ani nie pomijały zdarzeń, które nadejdą później |
| potok | wynik jego ostatniego etapu |

Szczegóły części. `next` po ostatnich wierszach wyniku, który skracał wiersze,
odczytuje skrócone wiersze w całości, tak jak `next full`; `next full` dla
wyniku bez skróconych wierszy daje pustą część, a dla kontynuacji zapytania z
kursorem (`logs`) kończy się błędem `E_INVALID_VALUE`. Wynik pokazujący wybór
wierszy (`show: errors`) jest kontynuowany od pierwszego wiersza pliku, więc
jego części powtarzają wiersze już pokazane. `run`, którego `stdout` i
`stderr` są oba skrócone, kontynuuje jeden strumień po drugim. Gdy jeden
program ma kilka etapów skracających wyjście (na przykład dwa `run`),
kontynuowany jest pierwszy z nich w kolejności programu; pozostałe są dostępne
przez własne pliki wyjściowe. `next` uruchamia zapytanie z kursorem (`logs`)
ponownie z opcjami pierwszego wywołania (`session`, `base`, `timeout`).
Kontynuacja należy do sesji, która ją utworzyła: inna sesja otrzymuje
`E_CONTINUATION_EXPIRED`. Wynik większy niż `continuation.max_size` nie jest
zachowywany; odpowiedź nie ma `continuation`, a uwaga wyjaśnia powód.

`next` jest poleceniem odczytu: niczego nie zmienia, nie zapisuje wpisu
dziennika i nie kosztuje skutku w `E_ONE_EFFECT`. Odpowiedź zawiera własne
`continuation` z tym samym `id`, dopóki coś pozostaje; ostatnia część ma
`remaining: 0` i nie ma wskazówki. Czas życia i miejsce zachowanych wyników
ustalają `continuation.max_age` (1 godzina), `continuation.max_count` (50) i
`continuation.max_size` (256 MiB); najstarsza kontynuacja jest usuwana
najpierw, a wyniki są usuwane po zakończeniu sesji konsoli.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `id` | `int \| field \| text` | najnowsza kontynuacja | Kontynuacja do odczytu, `c3`; także liczba (`next 200`) lub słowa `all` i `full`. |
| `count` | `int \| field \| text` | rozmiar pierwszej odpowiedzi | Jednostki w części. |
| `all` | `bool` | `false` | Odczytuje wszystko, co zostało. |
| `full_lines` | `bool` | `false` | Odczytuje w całości wiersze ostatnio pokazanej części, które zostały skrócone. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `any` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Odczyt następnej części najnowszego skróconego wyniku; bez zachowanego wyniku
wywołanie kończy się błędem `E_CONTINUATION_EXPIRED`:

```nurt example
next
```

```text expected
E_CONTINUATION_EXPIRED
```

Odczyt 200 jednostek kontynuacji `c3`:

```nurt example
next c3 200
```

Odczyt wszystkiego, co zostało z najnowszej kontynuacji, i policzenie tego:

```nurt example
next all |> count
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_CONTINUATION_EXPIRED` | kontynuacja jest starsza niż `continuation.max_age`, została usunięta przez `continuation.max_count` lub `continuation.max_size` albo żadna nie jest zachowana; `next_steps` zawiera wywołanie, które ją utworzyło | uruchom ponownie wywołanie z `next_steps` |
| `E_INVALID_VALUE` | `count` jest mniejsze niż 1 albo identyfikator nie ma postaci `cN` | podaj liczbę co najmniej 1 i identyfikator, taki jak `c3` |

## Powiązane {#related}

`run`, `search`, `find`, `list`, `logs`, `help skill: "large-results"`.
