---
command: journal.purge
group: journal
summary: Zwalnia wpisy dziennika i miejsce, które zajmują.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# journal.purge

## Składnia {#synopsis}

```text
journal.purge [name: value]...
```

## Opis {#description}

`journal.purge` natychmiast zwalnia wpisy dziennika wraz z miejscem, które
zajmują ich dane. Bez `up_to` i `id` zwalniany jest każdy wpis wybranych
woluminów; `up_to` zwalnia wpisy do podanego numeru operacji włącznie; `id`
zwalnia dokładnie wymienione wpisy i zachowuje starsze.

Przypięte wpisy są zwalniane również: przypięcie chroni wpis przed wygaśnięciem
z powodu kroków, wieku i rozmiaru, a nie przed jawnym zwolnieniem. Operacja
obejmująca kilka woluminów jest zwalniana na wszystkich, aby żaden wpis nie
pozostał częściowo możliwy do cofnięcia.

Zwolniony wpis przyjmuje stan `expired` z przyczyną `purge`; dla `history`
pozostaje znacznik usunięcia z jego metrykami. Zwalniane są wpisy zatwierdzone,
częściowe i cofnięte; wpisy już wygasłe, odzyskiwane lub uszkodzone nie są
zmieniane. Zwolnienie jest zapisywane w dzienniku audytu konta; nie tworzy wpisu
dziennika i nie zużywa kroku.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `volume` | `path \| enum(all)` | `all` | Wolumin, którego dziennik jest czyszczony, albo wszystkie woluminy. |
| `up_to` | `int` | brak | Zwalnia wszystkie wpisy do tego numeru operacji włącznie. |
| `id` | `list<int>` | brak | Zwalnia tylko te wpisy, zachowując starsze. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

Rekord `effect` rodzaju `journal.purge` wymienia numery zwolnionych wpisów w
`after.released` i ich rozmiar logiczny w `after.bytes`; `summary.released`
zlicza je, a `irreversible` zawiera `purged`, gdy zwolniono co najmniej jeden
wpis.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `journal` |
| Skutek | `local` |
| Odwracalność | `irreversible` |
| Implementacja | `builtin` |

Zwolnionych wpisów nie można już cofnąć; wynik zawiera znacznik
nieodwracalności `purged`.

## Przykłady {#examples}

Zwolnij każdy wpis każdego woluminu:

```nurt example
journal.purge
```

Zwolnij wpisy do operacji 120 na woluminie projektu:

```nurt example
journal.purge volume: "/home/user/project" up_to: 120
```

Zwolnij dwa duże wpisy i zachowaj resztę:

```nurt example
journal.purge id: [131, 134]
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | żaden wolumin dziennika nie zawiera `volume` | pomiń `volume` albo wskaż ścieżkę na woluminie z dziennikiem |
| `E_INVALID_VALUE` | podano jednocześnie `up_to` i `id` albo `id` nie wymienia żadnego wpisu | podaj jeden z nich |
| `E_IO` | nie można zmienić katalogu dziennika | sprawdź uprawnienia katalogu dziennika |

## Powiązane {#related}

`journal.status`, `journal.limits`, `journal.pin`, `history`.
