---
name: large-results
description: Ograniczanie dużych wyników przez stronicowanie, wybór pól, zliczanie, podsumowania i zapisane wyjście.
commands: [find, filter, select, count, summarize, group, first, sort, run, search, next]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/large-results.md -->
<!-- Responsibility:   Documents large-results. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# duże wyniki {#large-results}

## Stronicowanie {#paging}

Wynik jest ograniczony parametrem wywołania `limit`, wartością domyślną jest 50 rekordów, a odpowiedź zawiera `cursor`, który kontynuuje wynik w następnym wywołaniu. Program jest taki sam w każdym wywołaniu; zmienia się tylko `cursor`. Limit utrzymuje odpowiedź w budżecie wywołującego bez zmiany skutku programu.

## Kontynuacja skróconego wyniku {#continuing-a-shortened-result}

Odpowiedź, która pomija wiersze, rekordy lub końce długich wierszy, zawiera pole `continuation` i kończy się wskazówką "more: next". Wywołanie `next` zwraca kolejną część, w tym samym typie rekordu co pierwsza odpowiedź, aż `remaining` wyniesie 0; program nie jest uruchamiany ponownie, a części są spójne z pierwszą odpowiedzią.

```nurt
next
next 200
next c3 200
next all
next full
```

`next c3` kontynuuje kontynuację `c3`, gdy otwartych jest kilka wyników, `next 200` odczytuje 200 jednostek, `next all` wszystko, co zostało, a `next full` w całości wiersze ostatniej części, które zostały obcięte. Część jest wynikiem jak każdy inny: `next |> filter severity == "error"`. Zachowane wyniki wygasają po `continuation.max_age` (1 godzina), a limity `continuation.max_count` (50) i `continuation.max_size` (256 MiB) usuwają najpierw najstarsze; wygasła kontynuacja kończy się błędem `E_CONTINUATION_EXPIRED` i pierwotnym wywołaniem w `next_steps`.

## Mniej pól {#fewer-fields}

Parametr wywołania `fields` wymienia pola każdego rekordu do zwrócenia. `select` robi to samo wewnątrz programu i dodatkowo dodaje pola obliczane, dzięki czemu kolejne etapy i wynik zawierają tylko potrzebne pola.

```nurt
find "/srv/app" glob: "**/*.go" |> select path size
find "/srv/app" glob: "**/*.go" |> select path total: size * 2
```

## Zliczanie i podsumowywanie {#counting-and-summarizing}

`count` zwraca liczbę rekordów, a `summarize` oblicza liczbę, sumę, minimum, maksimum i średnią pól, opcjonalnie dla każdej grupy przez `by`. `group` zlicza rekordy dla każdej wartości pola. Te etapy odpowiadają na pytanie o duży zbiór bez zwracania samego zbioru.

```nurt
find "/srv/app" |> count
find "/srv/app" |> summarize size by: type
search "deprecated" "/srv/app/src" |> group path
```

## Wczesne zawężanie {#narrowing-early}

Warunek umieszczony blisko źródła zmniejsza ilość danych przetwarzanych przez kolejne etapy. `first` z `sort` zwraca największe lub najnowsze wpisy, a `search` z `output: counts` lub `output: files` zwraca liczby albo nazwy plików zamiast wierszy.

```nurt
find "/srv/app" |> sort size order: desc |> first 10
search "deprecated" "/srv/app/src" output: counts
```

## Zapisane wyjście {#saved-output}

Parametr wywołania `save_to` zapisuje pełny wynik do pliku i zwraca podsumowanie ze ścieżką pliku. Program, który zapisuje dużo wyjścia, może ograniczyć to, co wraca, przez `show: errors` lub `show: summary`, a `head` i `tail` zachowują pierwsze lub ostatnie wiersze.

```nurt
run ["make", "all"] cwd: "/srv/app" show: errors tail: 40
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
