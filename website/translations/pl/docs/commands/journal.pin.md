---
command: journal.pin
group: journal
summary: Chroni wpisy dziennika przed wygaśnięciem według kroków, wieku i rozmiaru albo zwalnia tę ochronę.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# journal.pin

## Składnia {#synopsis}

```text
journal.pin <id>... [name: value]...
```

## Opis {#description}

`journal.pin` chroni wpisy dziennika przed wygaśnięciem z powodu limitu kroków,
limitu wieku i budżetu rozmiaru przez podany czas. Przypięty wpis pozostaje
możliwy do cofnięcia po nowszych operacjach, upływie czasu i innych wpisach,
które by go wygasiły. Przypięcie nigdy nie chroni wpisu przed odzyskaniem
miejsca, gdy wolumin potrzebuje miejsca dla użytkownika: wolne miejsce dla
użytkownika zawsze ma priorytet.

Bez `until` przypięcie trwa `journal.pin_max_age` (domyślnie 24 godziny).
Ponowne przypięcie przypiętego wpisu zastępuje jego przypięcie. `release: true`
usuwa przypięcia; zwolnione wpisy, które w tej chwili przekraczają limit,
wygasają od razu i są wymienione w `expired`.

Przypięcie jest zapisywane poza wpisem, w katalogu dziennika każdego woluminu,
który przechowuje część operacji. Przypięcie nie zmienia ani wpisu, ani jego
skrótu, nie tworzy wpisu dziennika i nie zużywa numeru operacji ani kroku.

Wpisy, które nie istnieją, wygasły albo zostały cofnięte, są wymienione w
`not_available` i pozostawione bez zmian. Wynik nie podaje, kiedy przypięcie się
kończy; `history` pokazuje, czy operacja jest przypięta.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `id` | `list<int>` | brak | Wpisy do przypięcia lub zwolnienia. |
| `release` | `bool` | `false` | Zwalnia przypięcie zamiast je ustawiać. |
| `until` | `duration` | skonfigurowany maksymalny wiek przypięcia | Czas trwania przypięcia. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `journal` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |
| Warunki odwracalności | Przypięte wpisy nadal można zwolnić, gdy na woluminie zabraknie miejsca. |

## Przykłady {#examples}

Zachowaj możliwość cofnięcia operacji 18 przez domyślny czas:

```nurt example
journal.pin 18
```

Przypnij dwie operacje na trzy dni:

```nurt example
journal.pin 18 19 until: 3d
```

Zwolnij przypięcie operacji 18:

```nurt example
journal.pin 18 release: true
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_IO` | nie można zapisać pliku przypięcia woluminu albo dziennik woluminu wpisu jest niedostępny | zamontuj wolumin i sprawdź jego dziennik poleceniem `journal.status` |

## Powiązane {#related}

`history`, `journal.status`, `journal.limits`, `journal.purge`, `undo`.
