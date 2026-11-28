---
command: assert
group: core
summary: Zatrzymuje program z błędem E_ASSERT, chyba że rekordy spełniają jeden warunek.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# assert

## Składnia {#synopsis}

```text
any |> assert (count: <range> | paths: <range> | exit_code: <range> | all: <expression> | any: <expression> | empty: <bool> | contains: <text> | if_match: <text>)
```

## Opis {#description}

`assert` odczytuje całe wejście, sprawdza dokładnie jeden warunek i, gdy
warunek jest spełniony, przekazuje rekordy dalej bez zmian, z ich nagłówkiem
i kolejnością. Gdy warunek nie jest spełniony, program zatrzymuje się z błędem
`E_ASSERT`; błąd zawiera `expected`, `actual` oraz `sample` obejmujący do pięciu
rekordów lub ścieżek.

| Warunek | Jest spełniony, gdy |
|---|---|
| `count` | liczba rekordów mieści się w zakresie |
| `paths` | liczba różnych wartości `path` mieści się w zakresie |
| `exit_code` | każdy rekord ma `exit_code` w zakresie; kończy się niepowodzeniem, gdy nie uruchomiono żadnego programu |
| `all` | wyrażenie ma wartość `true` dla każdego rekordu |
| `any` | wyrażenie ma wartość `true` dla co najmniej jednego rekordu |
| `empty` | wejście nie ma rekordów (`true`) albo ma rekordy (`false`) |
| `contains` | pole tekstowe rekordu zawiera ten tekst |
| `if_match` | każdy rekord wejściowy ma podany `etag`; kończy się niepowodzeniem bez wejścia |

Przed skutkiem (etap `3`) nieudana asercja niczego nie zmienia. Po skutku
(etap `5a`), na przykład po `run`, skutek pozostaje, raport jest kompletny,
a błąd ma etap `5a`; skutek wycofuje wyłącznie `verify`.

Zakres zapisuje się jako `5`, `1..5`, `40..` lub `..9`; obie granice są
włączone.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `count` | `range` | jeden z `condition` | Oczekiwana liczba rekordów. |
| `paths` | `range` | jeden z `condition` | Oczekiwana liczba różnych ścieżek. |
| `exit_code` | `range` | jeden z `condition` | Oczekiwany kod zakończenia uruchomionych programów. |
| `all` | `expression` | jeden z `condition` | Warunek, który spełnia każdy rekord. |
| `any` | `expression` | jeden z `condition` | Warunek, który spełnia co najmniej jeden rekord. |
| `empty` | `bool` | jeden z `condition` | Czy nie ma rekordów. |
| `contains` | `text` | jeden z `condition` | Tekst, który zawierają rekordy. |
| `if_match` | `text` | jeden z `condition` | Oczekiwany etag pliku wejściowego. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `any` | `same` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `check` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Sprawdzenie, że wyszukiwanie znalazło dokładnie dwa pliki, zanim zostaną zmienione:

```nurt example
search /oldName\(/ "/home/user/project/src" |> assert paths: 2
```

Zatrzymanie, gdy liczba plików różni się od oczekiwanej:

```nurt example
find "/home/user/project" glob: "*.md" |> assert count: 2
```

Sprawdzenie kodu zakończenia programu po jego uruchomieniu:

```nurt example
run ["go", "test", "./..."] cwd: "/home/user/project" |> assert exit_code: 0
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_ASSERT` | warunek nie jest spełniony; `expected`, `actual` i `sample` go opisują | popraw wejście lub oczekiwaną wartość |
| `E_MISSING_PARAM` | nie podano żadnego warunku | podaj jeden z parametrów |
| `E_PARAM_CONFLICT` | podano więcej niż jeden warunek; `fix` wymienia każdy osobno | zostaw jeden warunek na `assert`, połącz kilka etapów w łańcuch |
| `E_TYPE` | warunek ma wartość innego typu albo `all`/`any` daje wartość inną niż `bool` | użyj typu parametru |

## Powiązane {#related}

`verify`, `count`, `filter`.
