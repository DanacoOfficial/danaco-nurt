---
command: decompress
group: archive
summary: Dekompresuje pojedynczo skompresowane pliki.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# decompress

## Składnia {#synopsis}

```text
decompress <path>... [name: value]...
```

## Opis {#description}

`decompress` zapisuje zdekompresowaną postać każdego pliku obok niego.
Algorytm (gzip, zstd, xz, bzip2 lub lz4) jest ustalany na podstawie zawartości
pliku; nazwa wyniku jest nazwą bez rozszerzenia kompresji. Rozszerzenia `.gz`,
`.zst`, `.xz`, `.bz2`, `.lz4` są usuwane, a `.tgz`, `.tzst`, `.txz`, `.tbz2`
zamieniane na `.tar`. Rozszerzenia są porównywane bez względu na wielkość liter.
Plik, którego nazwa nie ma żadnego z tych rozszerzeń, kończy się błędem
`E_INVALID_VALUE`, ponieważ nazwy wyniku nie można wyprowadzić; należy najpierw
zmienić nazwę pliku.

Strumienie złożone z kilku składników lub ramek są dekompresowane do
konkatenacji składników. Zawartość jest przesyłana strumieniowo do pliku
pośredniego transakcji, więc zużycie pamięci nie zależy od rozmiaru pliku.
Wynik jednego pliku jest ograniczony do 1 TiB; strumień, który dałby więcej,
kończy się błędem `E_LIMIT` i nic nie jest zapisywane.

Przy `keep: false` skompresowane pliki są usuwane w tej samej transakcji, po
zapisaniu każdego wyniku. Niepowodzenie pozostawia je na miejscu.

Istniejący wynik kończy się błędem `E_EXISTS` przed wprowadzeniem jakiejkolwiek
zmiany; polecenie nigdy nie nadpisuje pliku. W postaci etapu rekordy katalogów
i innych wpisów, które nie są zwykłymi plikami, są pomijane.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `list<path>` | brak | Pliki do zdekompresowania w postaci źródła. |
| `keep` | `bool` | `true` | Zachowuje skompresowane pliki. |

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

Zdekompresowanie pliku gzip z jego zachowaniem.

```nurt example
decompress "/srv/app/app.log.gz"
```

```text expected
op 51  decompress  ok
targets 1 file
  + /srv/app/app.log
```

Zdekompresowanie pliku zstd i usunięcie skompresowanego pliku.

```nurt example
decompress "/srv/app/data.csv.zst" keep: false
```

```text expected
op 52  decompress  ok
targets 1 file
  + /srv/app/data.csv
  - /srv/app/data.csv.zst
```

Zdekompresowanie skompresowanego pliku tar do pliku tar, a następnie wyświetlenie jego wpisów.

```nurt example
decompress "/srv/app/backup.tgz"
```

```text expected
op 53  decompress  ok
targets 1 file
  + /srv/app/backup.tar
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Postać źródła nie ma `path`. | Podaj pliki do zdekompresowania albo przekaż rekordy `entry`. |
| `E_PARAM_CONFLICT` | `path` podano w postaci etapu. | Usuń `path` albo rozpocznij potok od `decompress`. |
| `E_INVALID_VALUE` | Nazwa nie ma rozszerzenia kompresji (`problem: unknown_extension`) albo ścieżka postaci źródła nie jest zwykłym plikiem (`problem: not_a_file`). | Zmień nazwę pliku tak, aby kończyła się rozszerzeniem kompresji. |
| `E_FORMAT` | Zawartość nie jest danymi gzip, zstd, xz, bzip2 ani lz4 (`problem: not_compressed`) albo strumień jest uszkodzony lub kończy się przedwcześnie. | Sprawdź plik albo przywróć go ze źródła. |
| `E_LIMIT` | Zdekompresowany wynik przekracza limit (`problem: decompressed_size`). | Sprawdź, czy plik nie jest bombą kompresyjną; odczytaj jego część za pomocą `read ... decompress: auto`. |
| `E_EXISTS` | Plik wyniku istnieje albo dwa źródła mają tę samą nazwę wyniku (`problem: output_exists`). | Usuń istniejący plik lub zmień jego nazwę. |
| `E_NOT_FOUND` | Plik nie istnieje. | Sprawdź ścieżkę. |
| `E_STALE` | Źródło zmieniło się po sporządzeniu planu. | Powtórz wywołanie. |
| `E_PERMISSION` | Pliku nie można odczytać albo wyniku nie można zapisać. | Sprawdź uprawnienia i wolne miejsce. |
| `E_IO` | Pliku nie można odczytać albo wyniku nie można zapisać. | Sprawdź uprawnienia i wolne miejsce. |
| `E_NO_SPACE` | Pliku nie można odczytać albo wyniku nie można zapisać. | Sprawdź uprawnienia i wolne miejsce. |
| `E_CANCELLED` | Wywołanie zostało anulowane albo osiągnęło limit czasu. | Powtórz wywołanie z dłuższym `timeout`. |
| `E_TIMEOUT` | Wywołanie zostało anulowane albo osiągnęło limit czasu. | Powtórz wywołanie z dłuższym `timeout`. |

## Powiązane {#related}

`compress` odwraca działanie polecenia. `archive.extract` rozpakowuje
archiwa. `read ... decompress: auto` odczytuje skompresowany plik bez
zapisywania zdekompresowanej postaci.
