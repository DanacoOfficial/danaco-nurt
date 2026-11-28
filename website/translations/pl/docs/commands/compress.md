---
command: compress
group: archive
summary: Kompresuje pliki osobno za pomocą gzip, zstd, xz, bzip2 lub lz4.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# compress

## Składnia {#synopsis}

```text
compress <path>... [name: value]...
```

## Opis {#description}

`compress` zapisuje skompresowaną postać każdego pliku obok niego. Nazwa wyniku
jest nazwą pliku z dopisanym rozszerzeniem algorytmu: `.gz`, `.zst`, `.xz`,
`.bz2` lub `.lz4`. Nazwa, która ma już to rozszerzenie, jest rozszerzana ponownie,
więc `data.tar` staje się `data.tar.gz`.

Każdy plik jest osobnym strumieniem; aby spakować kilka plików do jednego
archiwum, należy użyć `archive.create`. Zawartość jest przesyłana strumieniowo
do pliku pośredniego transakcji, więc zużycie pamięci nie zależy od rozmiaru
pliku. Strumienie zstd i lz4 zawierają sumę kontrolną zawartości.

Przy `keep: false` źródła są usuwane w tej samej transakcji, po zapisaniu
każdego wyniku. Niepowodzenie pozostawia źródła na miejscu.

Istniejący wynik kończy się błędem `E_EXISTS` przed wprowadzeniem jakiejkolwiek
zmiany; polecenie nigdy nie nadpisuje pliku. W postaci etapu rekordy katalogów
i innych wpisów, które nie są zwykłymi plikami, są pomijane. Ścieżka podana
dwa razy jest kompresowana jeden raz.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `list<path>` | brak | Pliki do skompresowania w postaci źródła. |
| `algorithm` | `enum(gzip\|zstd\|xz\|bzip2\|lz4)` | `gzip` | Algorytm kompresji. |
| `keep` | `bool` | `true` | Zachowuje oryginalne pliki. |
| `level` | `int` | brak | Poziom kompresji algorytmu. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none`, `entry` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

Wpisy szczególne: `nothing_matched` dla pustego wejścia.

## Przykłady {#examples}

Skompresowanie pliku dziennika za pomocą gzip z jego zachowaniem.

```nurt example
compress "/srv/app/app.log"
```

```text expected
op 41  compress  ok
targets 1 file
  + /srv/app/app.log.gz
```

Skompresowanie dwóch plików za pomocą zstd na poziomie 19 i usunięcie oryginałów.

```nurt example
compress "/srv/app/a.csv" "/srv/app/b.csv" algorithm: zstd level: 19 keep: false
```

```text expected
op 42  compress  ok
targets 2 files
  + /srv/app/a.csv.zst
  + /srv/app/b.csv.zst
  - /srv/app/a.csv
  - /srv/app/b.csv
```

Skompresowanie starych dzienników katalogu za pomocą xz.

```nurt example
find "/srv/app/logs" glob: "*.log" type: file |> compress algorithm: xz
```

```text expected
op 43  compress  ok
targets 1 file
  + /srv/app/logs/2026-01.log.xz
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Postać źródła nie ma `path`. | Podaj pliki do skompresowania albo przekaż rekordy `entry`. |
| `E_PARAM_CONFLICT` | `path` podano w postaci etapu. | Usuń `path` albo rozpocznij potok od `compress`. |
| `E_UNKNOWN_VALUE` | `algorithm` nie nazywa żadnego obsługiwanego algorytmu. | Użyj jednego z `gzip`, `zstd`, `xz`, `bzip2`, `lz4`. |
| `E_INVALID_VALUE` | `level` jest poza zakresem algorytmu (`problem: level_out_of_range`) albo ścieżka postaci źródła nie jest zwykłym plikiem (`problem: not_a_file`). | Użyj poziomu z zakresu wskazanego w tabeli; pliki katalogu skompresuj za pomocą `find ... type: file \|> compress`. |
| `E_EXISTS` | Plik wyniku istnieje (`problem: output_exists`). | Usuń istniejący plik lub zmień jego nazwę. |
| `E_NOT_FOUND` | Plik nie istnieje. | Sprawdź ścieżkę. |
| `E_STALE` | Źródło zmieniło się po sporządzeniu planu. | Powtórz wywołanie. |
| `E_PERMISSION` | Pliku nie można odczytać albo wyniku nie można zapisać. | Sprawdź uprawnienia i wolne miejsce. |
| `E_IO` | Pliku nie można odczytać albo wyniku nie można zapisać. | Sprawdź uprawnienia i wolne miejsce. |
| `E_NO_SPACE` | Pliku nie można odczytać albo wyniku nie można zapisać. | Sprawdź uprawnienia i wolne miejsce. |
| `E_CANCELLED` | Wywołanie zostało anulowane albo osiągnęło limit czasu. | Powtórz wywołanie z dłuższym `timeout`. |
| `E_TIMEOUT` | Wywołanie zostało anulowane albo osiągnęło limit czasu. | Powtórz wywołanie z dłuższym `timeout`. |

## Powiązane {#related}

`decompress` odwraca działanie polecenia. `archive.create` pakuje kilka plików
do jednego archiwum. `read ... decompress: auto` odczytuje skompresowany plik
bez zapisywania jego zdekompresowanej postaci.
