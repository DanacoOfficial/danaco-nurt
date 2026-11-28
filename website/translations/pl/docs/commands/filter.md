---
command: filter
group: core
summary: Zachowuje rekordy spełniające warunek.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# filter

## Składnia {#synopsis}

```text
any |> filter <condition>
```

## Opis {#description}

`filter` oblicza `condition` dla każdego rekordu wejścia i przekazuje dalej
rekordy, dla których wynikiem jest `true`. Warunek, którego wynikiem jest
`null`, na przykład porównanie z polem bez wartości, jest traktowany jak
`false`. Każda wartość inna niż `bool` zatrzymuje program z błędem `E_TYPE`.

Gdy rodzaje rekordów wejściowych są znane, wyrażenie, którego żaden rekord nie
może spełnić, jest sprawdzane przed uruchomieniem programu, przez `check` i
przez wiązanie każdego uruchomienia, z błędem, który obliczenie zgłosiłoby dla
każdego rekordu mającego wartość w tym polu (ten sam kod, komunikat i
`expected`). Sprawdzenie obejmuje pole znanego typu i literał, w dowolnej
kolejności, w następujących miejscach:

| Wyrażenie | Błąd |
|---|---|
| `size > "abc"`, `modified < "never"`, `size == "abc"`: tekst, który nie tworzy wartości typu pola `size`, `duration`, `instant`, `path`, `glob` lub `key` | `E_INVALID_VALUE`, z typem w `expected` |
| `<`, `<=`, `>`, `>=` z literałem innej rodziny typów (`size > 5`, `name > 5`, `flags > 5`, `raw_name > "abc"`) | `E_TYPE` |
| `in` z zakresem i polem, które nie jest `int` (`size in 1..5`), oraz `in` z prawym operandem, który nie jest listą, zakresem ani mapą (`5 in size`, `name in "abc"`) | `E_TYPE` |
| `+ - * /` na typach bez wyniku (`size + "abc"`, `size - 5`, `modified - 5`, `name - 1`) | `E_TYPE` |
| `~` i `!~` z polem, które nie jest tekstem (`flags ~ /a/`) | `E_TYPE` |
| Wywołanie funkcji wbudowanej, której argumentem jest pole typu, którego funkcja nie przyjmuje (`lower(size)`, `len(exists)`) | `E_TYPE` |
| Wywołanie wyłącznie z argumentami literalnymi, które funkcja odrzuca (`ago("abc")`, `len()`) | błąd funkcji |

Błąd jest umieszczany przy literale, przy wywołaniu albo przy całej operacji,
gdy oba operandy są polami. Listy, mapy, bajty i zakresy uczestniczą ze swoją
rodziną typów: literał nigdy nie jest do nich konwertowany, więc `flags > 5`
jest odrzucane, natomiast `==`, `!=` i `in` z listą lub mapą nigdy nie zgłaszają
błędu na ich elementach i nie są sprawdzane. `==` i `!=` z literałem innego
typu nie są błędem i dają `false` lub `true`.

Sprawdzane jest tylko to, co nie zależy od wartości pola. Przepełnienie i
dzielenie przez zero, operand będący samodzielnie obliczanym wyrażeniem, pole
typu `any` (na przykład `value` rekordu konfiguracji) oraz pole rodzaju
nieznanego podczas wiązania są sprawdzane podczas uruchomienia programu, rekord
po rekordzie, i zatrzymują go tymi samymi błędami. Wartość pola `null` daje
`false` lub `null`, jak dotychczas. Warunek jest sprawdzany tak, jak został
zapisany: `and` i `or` nie ukrywają przed `check` błędu w swoim prawym
operandzie.

Etap jest leniwy: odczytuje następny rekord wejścia dopiero wtedy, gdy
kolejny etap o niego poprosi, więc `filter … |> first 10` przestaje czytać
źródło po dziesiątym dopasowaniu. Kolejność rekordów, nagłówek strumienia i
wskazówka prezentacji źródła są zachowane. Puste wejście daje puste wyjście.

`filter` działa przed skutkiem, gdzie wybiera cele zmiany, oraz po skutku
(etap `5a`), gdzie odczytuje rekordy skutku, takie jak `exec`, `job`,
`test_result`, `finding` i `effect`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `condition` | `expression` | wymagany | Warunek obliczany dla każdego rekordu. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `any` | `same` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `transform` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Zachowanie plików większych niż 2 kB:

```nurt example
find "/home/user/project" |> filter size > 2kb
```

Zachowanie wierszy, które ustawiają `debug` w pliku konfiguracyjnym:

```nurt example
lines "/home/user/app/config.ini" |> filter text ~ /^\s*debug\s*=/
```

Zachowanie wyniku uruchomienia programu tylko wtedy, gdy zakończyło się ono niepowodzeniem:

```nurt example
run ["go", "vet", "./..."] cwd: "/home/user/project" |> filter exit_code != 0
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | brakuje `condition` | zapisz warunek po `filter` |
| `E_TYPE` | warunek daje wartość inną niż `bool` albo `<`, `<=`, `>` lub `>=` porównuje pole z literałem innej rodziny typów | porównaj pole, na przykład `size > 1mb` zamiast `size`, i zapisz literał typu tego pola |
| `E_INVALID_VALUE` | tekst porównywany z polem `size`, `duration` lub `instant` nie tworzy wartości tego typu; `expected` wskazuje typ | zapisz poprawną wartość, na przykład `"1mb"`, `"5m"` lub `"2026-11-01"` |
| `E_UNKNOWN_FIELD` | warunek wskazuje pole, którego rekordy nie mają; `expected` wymienia ich pola | użyj jednego z wymienionych pól |

## Powiązane {#related}

`select`, `sort`, `first`, `assert`, `count`.
