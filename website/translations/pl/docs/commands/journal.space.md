---
command: journal.space
group: journal
summary: Raportuje wolne miejsce, miejsce możliwe do odzyskania z dziennika i rezerwę każdego woluminu.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# journal.space

## Składnia {#synopsis}

```text
journal.space [name: value]...
```

## Opis {#description}

Raportuje jeden rekord `volume` na wolumin dziennika: wolne miejsce widziane
przez inne programy, miejsce, które dziennik zajmuje i mógłby zwolnić, oraz
progi chroniące miejsce użytkownika. Bez `volume` polecenie raportuje każdy
wolumin znany usłudze miejsca; z `volume` raportuje wolumin, który zawiera
ścieżkę.

Wolne miejsce `F` to miejsce dostępne dla konta po zajęciu swojej części przez
dziennik i po uwzględnieniu limitu konta, gdy limity obowiązują (`statvfs` w
systemie Linux, `volumeAvailableCapacityKey` w systemie macOS,
`GetDiskFreeSpaceExW` w systemie Windows). Miejsce możliwe do wyczyszczenia
raportowane przez macOS nie jest wliczane.

Progi wynikają z pojemności `T` woluminu albo z limitu konta, gdy jest mniejszy:

| Próg | Klucz | Wartość domyślna |
|---|---|---|
| Rezerwa `R` | `space.reserve` | 15 % `T`, ograniczone do `space.reserve_min` (5 GiB) i `space.reserve_max` (50 GiB); przy limicie konta nie więcej niż 15 % limitu |
| Próg miękki `S` | `space.soft` | `R` plus większa z wartości: 5 % `T` i 2 GiB, nie więcej niż `R + 20 GiB`; zawsze powyżej `R` |
| Margines `m` | `space.margin` | mniejsza z wartości: 1 % `T` i 1 GiB |
| Histereza `h` | brak | 1 % `T` |

Dysk 1 TB ma `R` = 50 GiB i `S` = 70 GiB; dysk 32 GB ma `R` = 5 GiB i
`S` = 7 GiB.

Progi określają stan woluminu:

| Stan | Warunek | Zachowanie dziennika |
|---|---|---|
| normal | `F ≥ S` | poprzednia zawartość jest zachowywana; wpisy wygasają tylko według swoich limitów |
| soft | `R ≤ F < S` | poprzednia zawartość jest zachowywana tylko wtedy, gdy po zapisie `F − extra ≥ R + m`; usługa zwalnia stare wpisy w tle, aż `F ≥ S + h` |
| hard | `F < R` | poprzednia zawartość nie jest zachowywana (`no_space_for_copy`); usługa zwalnia wpisy od razu, aż `F ≥ R + m`, duże wpisy częściowo, od największych plików |
| emergency | `F < R / 2` | jak hard, ze zwalnianiem na każdym rdzeniu procesora; polecenia nie czekają na usługę |

Przed zmianą dziennik zachowuje poprzednią zawartość celu tylko wtedy, gdy
wolumin się kwalifikuje, usługa miejsca działa, operacja mieści się w
`journal.max_op_size` i `journal.max_size`, a po zapisie spełnione są oba
warunki `F − extra ≥ R + m` i `bytes_held + extra ≤ journal.max_size`, gdzie
`extra` to miejsce, które dziennik zajmuje dodatkowo. W przeciwnym razie wpisy
wygasłe, a następnie stare wpisy są zwalniane w ramach `space.sync_budget_ms`
(1500 ms); gdy to nie wystarcza, cel jest zmieniany bez zachowania, a wynik
oznacza to jako nieodwracalne. Wpisy zwalniane dla miejsca są wybierane według
liczby bajtów zwolnionych na sekundę usuwania; przypięte wpisy są zwalniane dla
miejsca tak samo jak każdy inny wpis.

Gwarancje:

- Dziennik nigdy nie powoduje niepowodzenia operacji użytkownika z braku
  miejsca.
- Gdy usługa miejsca nie działa, dziennik nie zachowuje nowych danych.
- Sam dziennik nigdy nie obniża wolnego miejsca poniżej rezerwy; inne programy
  mogą.
- Zapis, który potrzebuje nie więcej niż `F` w chwili początku, gdy `F ≥ R`,
  kończy się powodzeniem; w szczególności każdy zapis co najwyżej `R` bajtów.

Brak gwarancji:

- Zapis, który potrzebuje więcej niż `F`, ale nie więcej niż `F` plus bajty
  możliwe do odzyskania, kończy się powodzeniem tylko wtedy, gdy usługa zwolni
  wpisy, zanim zapisowi zabraknie miejsca; instalatory, które sprawdzają wolne
  miejsce z góry, widzą tylko `F`.
- Gdy zapis obniży `F` poniżej `R`, wolne miejsce wraca powyżej `R` po
  zakończeniu zwalniania; dla wpisów z milionami plików trwa to minuty.
- Zapis zewnętrzny większy niż `F` i szybszy niż zwalnianie może wyczerpać
  miejsce; rezerwa chroni zapisy do `R`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `volume` | `path` | brak | Tylko wolumin, który zawiera tę ścieżkę. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `volume` |

Rozmiary są logiczne: dane współdzielone z nazwami poza dziennikiem przez twarde
dowiązania liczą się jako zero, a w systemach plików, które zwalniają bloki
leniwie, `free` rośnie do kilku sekund po zwolnieniu.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `journal` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Zaraportuj wolne miejsce i udział dziennika każdego woluminu dziennika:

```nurt example
journal.space
```

Zaraportuj wolumin katalogu domowego:

```nurt example
journal.space volume: "/home/user"
```

Zaraportuj wolumin, który zawiera katalog projektu; ścieżka wybiera jego
wolumin i nie musi być punktem montowania:

```nurt example
journal.space volume: "/home/user/projects/app"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | żaden wolumin nie zawiera ścieżki | podaj istniejącą ścieżkę albo pomiń `volume` |
| `E_SERVICE_NOT_RUNNING` | usługa miejsca nie działa, więc nie można zmierzyć żadnego woluminu | uruchom usługę miejsca, a następnie powtórz polecenie |
| `E_IO` | nie można odczytać wolnego miejsca ani indeksu dziennika | sprawdź wolumin i powtórz polecenie |

## Powiązane {#related}

`journal.status` raportuje wpisy, limity i ich źródła; `journal.purge`
natychmiast zwalnia wpisy; `journal.limits` zmienia limity kroków, wieku i
rozmiaru.
