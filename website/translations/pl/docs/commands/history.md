---
command: history
group: core
summary: Wyświetla operacje dziennika.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# history

## Składnia {#synopsis}

```text
history [<id>] [name: value]...
```

## Opis {#description}

`history` wyświetla operacje zapisane w dzienniku konta, od najnowszej.
Odczyty, podglądy i nieudane operacje wycofane, zanim zaczęły działać, nie mają
wpisu i nie są wyświetlane.

Każda operacja ma stan:

| Stan | Znaczenie |
|---|---|
| `available` | operację można cofnąć |
| `partial` | można cofnąć tylko niektóre elementy: pozostałe zostały zwolnione, aby odzyskać miejsce na dysku, albo cofnięte pojedynczo |
| `undone` | operację cofnięto |
| `expired` | operacji nie można już cofnąć; jej metryki pozostają na liście |

Operacje wciąż zapisywane lub wycofywane nie są wyświetlane. Operacja, która nie
przeszła kontroli integralności, jest wyświetlana jako `expired`.

Filtry łączą się ze sobą. `path` wybiera operacje, które zmieniły plik,
porównując ścieżki tak, jak robi to wolumin (wielkość liter i normalizacja
Unicode na woluminach niewrażliwych na wielkość liter); operacje wygasłe nie
przechowują listy elementów i nigdy nie pasują do `path` ani `deleted`. Bez
`limit` wyświetlanych jest najwyżej 20 operacji.

Przy `details: true` i `id` operacja, której nie ma w dzienniku albo która jest
wciąż zapisywana, kończy się błędem `E_JOURNAL_NOOP` zamiast pustego wyniku.
Przy `details: true` każdy wyświetlany rekord zawiera także `items`, czyli cele
operacji; lista jest pusta dla operacji `expired`, a bez `details` pole jest
nieobecne.

Wynik podaje, czy operację można cofnąć, nigdy do kiedy; limity przechowywania i
stan dziennika raportuje `journal.status`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `id` | `int` | brak | Operacja do opisania. |
| `limit` | `int` | `20` | Liczba wyświetlanych operacji. |
| `state` | `enum(any\|available\|partial\|expired\|undone)` | `any` | Stan operacji do wyświetlenia. |
| `before` | `int` | brak | Wyświetla tylko operacje starsze od tej. |
| `since` | `int` | brak | Wyświetla tylko operacje nowsze od tej. |
| `path` | `path` | brak | Wyświetla tylko operacje, które dotknęły tego pliku. |
| `deleted` | `bool` | `false` | Wyświetla tylko operacje, które usunęły pliki. |
| `details` | `bool` | `false` | Dołącza zmiany i różnice każdej operacji. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `operation` |

Każdy element `items` ma pola `path`, `action` (działanie na celu, na przykład
`create`, `modify` lub `rename`), `mode` (sposób przechowywania poprzedniej
zawartości, na przykład `clone` lub `none`), `before_hash` i `after_hash`
(nieobecne, gdy nie było zawartości), `undone` i `released`.

Postać tekstowa wyniku pokazuje kolumny `id`, `time`, `kind`, `state`, `files`,
`bytes` i `summary` oraz `items` przy `details: true`. Pozostałe pola operacji
(`action`, `irreversible`, `bulk`, `session`, `undo_of`, `redo_of`,
`undo_scope`, `ast_hash`, `manifest_sha256`, `chain` i `pinned`) pozostają w
postaci JSON i pojawiają się w tekście przy szczegółowości `detailed` albo gdy
żądanie wymienia je w `fields`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `journal` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetl najnowsze operacje:

```nurt example
history
```

Wyświetl operacje, które zmieniły jeden plik:

```nurt example
history path: "/home/user/app/main.go"
```

Opisz operację 18; wywołanie kończy się błędem, gdy operacji nie ma w dzienniku:

```nurt example
history 18 details: true
```

Wyświetl operacje, które usunęły pliki i nadal można je cofnąć:

```nurt example
history deleted: true state: available
```

Odpowiedź, która pomija rekordy, kończy się wskazówką „more: next” i zawiera pole
`continuation`; polecenie `next` zwraca kolejną część bez ponownego uruchamiania
polecenia.

```text
matched 90  shown 20  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_JOURNAL_NOOP` | przy `details: true` operacji `id` nie ma w dzienniku albo jest wciąż zapisywana | wyświetl operacje bez `id` |
| `E_INVALID_VALUE` | `limit` lub numer operacji jest mniejszy niż 1 | podaj liczbę co najmniej 1 |
| `E_IO` | nie można odczytać indeksu dziennika | sprawdź katalog stanu konta |

## Powiązane {#related}

`undo`, `redo`, `journal.status`, `journal.pin`.
