---
command: copy
group: core
summary: Kopiuje pliki i katalogi.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# copy

## Składnia {#synopsis}

```text
copy <from>... [<to>] (to: <path> | into: <path>) [name: value]...
entry |> copy [<to>] (to: <path> | into: <path>) [name: value]...
```

## Opis {#description}

`copy` kopiuje pliki, dowiązania symboliczne, punkty połączeń (junction)
i drzewa katalogów. Postać źródła przyjmuje ścieżki z `from`; postać etapu
przyjmuje rekordy `entry`, na przykład z `list` lub `find`. `to` i `into`
wzajemnie się wykluczają i wymagane jest jedno z nich.

`to` jest nową ścieżką kopii. Gdy `to` wskazuje istniejący katalog, kopia
jest umieszczana wewnątrz niego pod nazwą źródła, a wynik zgłasza `to_is_dir`.
`into` zawsze wskazuje katalog, który przyjmuje kopie; jest tworzony wraz
z brakującymi katalogami nadrzędnymi. Kilka źródeł wymaga katalogu: `into`
albo `to` wskazującego istniejący katalog. W postaci etapu `to` jest
obliczane dla każdego rekordu wejściowego.

Katalog jest przeglądany jeden raz, bez śledzenia dowiązań, i kopiowany
w całości w jednej operacji. Istniejący katalog docelowy otrzymuje zawartość
katalogu źródłowego; konflikty są wtedy rozstrzygane dla każdego pliku osobno.
Istniejący katalog nigdy nie jest zastępowany plikiem ani dowiązaniem,
a katalog nigdy nie jest kopiowany na plik.

`on_conflict` rozstrzyga, co dzieje się z istniejącym plikiem lub dowiązaniem
w miejscu docelowym: `overwrite` je zastępuje, `skip` je zachowuje i pomija
źródło, `error` kończy się niepowodzeniem przed wprowadzeniem jakiejkolwiek
zmiany. Dowiązanie symboliczne jest kopiowane jako dowiązanie o tym samym
celu; jego cel nie jest kopiowany.

`preserve: true` zachowuje bity uprawnień katalogów, z tym że katalog, którego
uprawnienia użytkownika nie obejmują odczytu, zapisu lub przeszukiwania, jest
tworzony z `rwx` dla użytkownika, ponieważ jego zawartość jest do niego
kopiowana później; wpis szczególny `attributes_changed` wskazuje ścieżkę i oba
tryby. Pozwala także dzienniku zachować czasy, uprawnienia i atrybuty
rozszerzone plików tam, gdzie system plików je obsługuje. `clone: auto` używa
klonów bloków, gdy źródło i miejsce docelowe leżą na tym samym woluminie, który
je obsługuje, a w przeciwnym razie kopii bajtów; `clone: always` kończy się
niepowodzeniem, gdy klony bloków nie są dostępne, a `clone: never` zawsze
kopiuje bajty.

Rekord wpisu z `etag` musi nadal mieć ten znacznik encji; zmieniony wpis kończy
się błędem `E_STALE` przed wprowadzeniem jakiejkolwiek zmiany. Puste wejście
niczego nie kopiuje i zgłasza `nothing_matched`.

| Kod szczególny | Znaczenie |
|---|---|
| `to_is_dir` | `to` wskazywało istniejący katalog, który otrzymał kopię |
| `case_collision` | miejsce docelowe różni się od istniejącej nazwy wyłącznie wielkością liter na woluminie, który nie rozróżnia wielkości liter |
| `nothing_matched` | wejście nie miało wpisów |

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `from` | `list<path>` | wymagany | Pliki i katalogi do skopiowania w postaci źródła. |
| `to` | `path` | jeden z `destination` | Ścieżka docelowa; istniejący katalog otrzymuje kopię. |
| `into` | `path` | jeden z `destination` | Katalog docelowy, tworzony, gdy brakuje. |
| `on_conflict` | `enum(overwrite\|skip\|error)` | `overwrite` | Zachowanie, gdy plik docelowy istnieje. |
| `preserve` | `bool` | `true` | Zachowuje czasy, uprawnienia i atrybuty. |
| `clone` | `enum(auto\|always\|never)` | `auto` | Używa klonów systemu plików, gdy są dostępne, zawsze lub nigdy. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| `source` | `none` | `effect` |
| `stage` | `entry` | `effect` |

Rekord `effect` ma jedną zmianę `change` na każdy utworzony katalog,
skopiowany plik i utworzone dowiązanie, z `from` ustawionym na źródło. Jego
`summary` zlicza `files`, `directories`, `links`, `overwritten` i `skipped`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Skopiowanie pliku obok niego samego:

```nurt example
copy "/srv/app/config.toml" "/srv/app/config.toml.bak"
```

```text expected
copy: 1 target, files 1
```

Skopiowanie drzewa katalogów do nowego katalogu:

```nurt example
copy "/srv/app/src" into: "/backup/2026-11-22"
```

Skopiowanie znalezionych plików dziennika z zachowaniem kopii, które już istnieją:

```nurt example
find "/srv/app/logs" glob: "*.log" |> copy into: "/archive/logs" on_conflict: skip
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | brak źródła albo brak zarówno `to`, jak i `into` | podaj `from` i miejsce docelowe |
| `E_PARAM_CONFLICT` | podano jednocześnie `to` i `into` albo kilka źródeł, a `to` nie jest istniejącym katalogiem | podaj jedno miejsce docelowe; dla kilku źródeł użyj `into` |
| `E_NOT_FOUND` | źródło lub katalog `to` nie istnieje | sprawdź ścieżkę albo użyj `into` |
| `E_EXISTS` | cel istnieje przy `on_conflict: error`, jest istniejącym katalogiem, ma inny rodzaj niż źródło albo dwa źródła mają jeden cel | wybierz inne miejsce docelowe lub inną wartość `on_conflict` |
| `E_INVALID_VALUE` | katalog kopiowany do samego siebie, źródło będące własnym celem albo `into`, które nie jest katalogiem | wybierz miejsce docelowe poza źródłem |
| `E_UNSUPPORTED` | `clone: always` bez klonów bloków albo urządzenie, potok lub gniazdo jako źródło | użyj `clone: auto`; wyłącz plik specjalny |
| `E_STALE` | wpis wejściowy zmienił się od chwili odczytu | odczytaj wpisy ponownie |
| `E_IN_USE` | Windows: plik jest otwarty w innym procesie; błąd wymienia procesy | zamknij tam plik |
| `E_PATH_RELATIVE` | ścieżka względna bez katalogu bazowego | podaj ścieżkę bezwzględną albo ustaw `base` |

## Powiązane {#related}

`move`, `rename`, `remove`, `create`, `find`, `list`.
