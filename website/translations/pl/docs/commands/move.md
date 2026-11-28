---
command: move
group: core
summary: Przenosi pliki i katalogi.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# move

## Składnia {#synopsis}

```text
move <from>... [<to>] (to: <path> | into: <path>) [name: value]...
entry |> move [<to>] (to: <path> | into: <path>) [name: value]...
```

## Opis {#description}

`move` przenosi pliki, dowiązania symboliczne, złącza i drzewa katalogów pod
inną ścieżkę. Postać źródłowa przyjmuje ścieżki z `from`; postać etapu
przyjmuje rekordy `entry`. `to` i `into` wykluczają się wzajemnie, a jeden z
nich jest wymagany.

`to` jest nową ścieżką wpisu. Gdy `to` wskazuje istniejący katalog, wpis jest
przenoszony do jego wnętrza pod własną nazwą, a wynik zgłasza `to_is_dir`.
`into` zawsze wskazuje katalog, który przyjmuje wpisy; jest tworzony wraz z
brakującymi katalogami nadrzędnymi. Kilka źródeł wymaga katalogu: `into` albo
`to` wskazującego istniejący katalog. W postaci etapu `to` jest oceniane dla
każdego rekordu wejściowego.

Każdy wpis jest przenoszony w całości: katalog wraz z zawartością, dowiązanie
jako dowiązanie. W obrębie jednego woluminu wpis jest przemianowywany. Między
woluminami dziennik kopiuje wpis i usuwa źródło po zakończeniu kopiowania;
wynik zgłasza `cross_device`. Przeniesienie, które zmienia tylko wielkość liter
w nazwie na woluminie nierozróżniającym wielkości liter, przemianowuje sam
wpis.

`on_conflict` rozstrzyga, co dzieje się z istniejącym plikiem lub dowiązaniem
w miejscu docelowym: `overwrite` zastępuje go, `skip` zachowuje go i pozostawia
źródło na miejscu, `error` kończy się błędem, zanim cokolwiek się zmieni.
Istniejący katalog nigdy nie jest zastępowany. Katalog nie może zostać
przeniesiony do samego siebie, a źródło wewnątrz innego źródła tego samego
wywołania jest odrzucane.

Rekord wpisu z `etag` musi nadal mieć ten znacznik encji; zmieniony wpis kończy
się błędem `E_STALE`, zanim cokolwiek się zmieni. Puste wejście niczego nie
przenosi i zgłasza `nothing_matched`.

| Kod uwagi | Znaczenie |
|---|---|
| `to_is_dir` | `to` wskazywało istniejący katalog, który przyjął wpis |
| `cross_device` | wpis został przeniesiony na inny wolumin jako kopiowanie i usunięcie |
| `case_collision` | miejsce docelowe różni się od istniejącej nazwy tylko wielkością liter na woluminie nierozróżniającym wielkości liter |
| `nothing_matched` | wejście nie zawierało wpisów |

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `from` | `list<path>` | wymagany | Pliki i katalogi do przeniesienia w postaci źródłowej. |
| `to` | `path` | jeden z `destination` | Ścieżka docelowa; istniejący katalog przyjmuje wpis. |
| `into` | `path` | jeden z `destination` | Katalog docelowy, tworzony, gdy brakuje. |
| `on_conflict` | `enum(overwrite\|skip\|error)` | `overwrite` | Zachowanie, gdy plik docelowy istnieje. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| `source` | `none` | `effect` |
| `stage` | `entry` | `effect` |

Rekord `effect` ma jedną zmianę `change` z akcją `rename` na każdy
przeniesiony wpis, z `path` ustawionym na źródło i `to` na miejsce docelowe,
oraz jedną zmianę `change` na każdy katalog utworzony dla `into`. Jego
`summary` zlicza `moved`, `overwritten`, `skipped` i `cross_device`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Przeniesienie pliku do istniejącego katalogu:

```nurt example
move "/home/user/report.txt" "/home/user/done"
```

```text expected
move: 1 target, moved 1, notable: to_is_dir
```

Przeniesienie dwóch plików do katalogu, który jeszcze nie istnieje:

```nurt example
move "/srv/a.log" into: "/srv/old"
```

Przeniesienie pliku na inny wolumin:

```nurt example
move "/data/export.csv" "/mnt/usb/export.csv"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | brak źródła albo brak `to` i `into` | podaj `from` i miejsce docelowe |
| `E_PARAM_CONFLICT` | podano jednocześnie `to` i `into` albo kilka źródeł, a `to` nie jest istniejącym katalogiem | podaj jedno miejsce docelowe; dla kilku źródeł użyj `into` |
| `E_NOT_FOUND` | źródło albo katalog z `to` nie istnieje | sprawdź ścieżkę albo użyj `into` |
| `E_EXISTS` | cel istnieje przy `on_conflict: error`, jest istniejącym katalogiem albo dwa źródła mają jeden cel | wybierz inne miejsce docelowe albo inną wartość `on_conflict` |
| `E_INVALID_VALUE` | katalog przeniesiony do samego siebie, źródło będące własnym celem albo źródło wewnątrz innego źródła | przenieś tylko katalog nadrzędny |
| `E_STALE` | wpis wejściowy zmienił się od chwili odczytu | odczytaj wpisy ponownie |
| `E_IN_USE` | Windows: wpis jest otwarty w innym procesie; błąd wymienia procesy | zamknij wpis w tym procesie |
| `E_PATH_RELATIVE` | ścieżka względna bez katalogu bazowego | podaj ścieżkę bezwzględną albo ustaw `base` |

## Powiązane {#related}

`rename`, `copy`, `remove`, `find`, `list`.
