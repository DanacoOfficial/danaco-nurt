---
command: hash
group: text
summary: Oblicza skróty zawartości plików.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# hash

## Składnia {#synopsis}

```text
hash <path>... [name: value]...
```

## Opis {#description}

`hash` oblicza skrót zawartości każdego pliku i zwraca go w polu `hash`
rekordu `entry`, w postaci `<algorithm>:<hex>`, na przykład
`sha256:ba7816bf...`. Część szesnastkowa jest taka sama jak wyjście
`sha256sum`, `sha512sum`, `sha1sum`, `md5sum` i `b3sum`; CRC-32 jest zapisywany
jako osiem cyfr szesnastkowych w kolejności bajtów big-endian, tak jak w
`cksum -a crc32b` i `7z h`.

W postaci źródłowej `hash` zwraca jeden rekord `entry` na ścieżkę, w kolejności
ścieżek. Dowiązanie jest haszowane według zawartości swojego celu, a jego rekord
opisuje samo dowiązanie. Katalog lub inny wpis, który nie jest zwykłym plikiem,
kończy się błędem `E_INVALID_VALUE`.

W postaci etapu `hash` ustawia pole `hash` każdego wejściowego rekordu `entry`
i zachowuje jego pozostałe pola. Rekordy katalogów i innych wpisów, które nie są
zwykłymi plikami, są przekazywane dalej bez zmian, bez pola `hash`. Puste
wejście daje puste wyjście.

Zawartość jest odczytywana we fragmentach, więc plik dowolnego rozmiaru jest
haszowany w stałej ilości pamięci. Przy więcej niż jednym wątku w limitach
wywołania pliki są haszowane równolegle; wyjście zachowuje kolejność wejścia.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `list<path>` | brak | Pliki do zahaszowania w postaci źródłowej. |
| `algorithm` | `enum(sha256\|sha512\|sha1\|md5\|blake3\|crc32)` | `sha256` | Algorytm skrótu. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none`, `entry` | `entry` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Zahaszuj jeden plik algorytmem SHA-256.

```nurt example
hash "/srv/app/release.txt"
```

```text expected
path                    hash
/srv/app/release.txt    sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
```

Zahaszuj dwa pliki algorytmem MD5.

```nurt example
hash "/srv/app/a.txt" "/srv/app/b.txt" algorithm: md5
```

```text expected
path              hash
/srv/app/a.txt    md5:900150983cd24fb0d6963f7d28e17f72
/srv/app/b.txt    md5:f96b697d7cb7938d525a2f31aaf161d0
```

Zahaszuj pliki z wykazu algorytmem BLAKE3 i zgrupuj pliki o równej zawartości.

```nurt example
find "/srv/app" type: file |> hash algorithm: blake3 |> group by: hash
```

Oblicz CRC-32 pliku.

```nurt example
hash "/srv/app/check.txt" algorithm: crc32
```

```text expected
path                  hash
/srv/app/check.txt    crc32:cbf43926
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Postać źródłowa nie ma `path`. | Podaj pliki do zahaszowania albo przekaż rekordy `entry`. |
| `E_PARAM_CONFLICT` | `path` podano w postaci etapu. | Usuń `path` albo rozpocznij potok od `hash`. |
| `E_UNKNOWN_VALUE` | `algorithm` nie wskazuje obsługiwanego algorytmu. | Użyj jednego z `sha256`, `sha512`, `sha1`, `md5`, `blake3`, `crc32`. |
| `E_INVALID_VALUE` | Ścieżka postaci źródłowej jest katalogiem albo nie jest zwykłym plikiem (`problem: not_a_file`). | Zahaszuj pliki katalogu poleceniem `find ... type: file \|> hash`. |
| `E_NOT_FOUND` | Plik nie istnieje. | Sprawdź ścieżkę. |
| `E_PERMISSION` | Nie można odczytać pliku. | Sprawdź uprawnienia pliku. |
| `E_IO` | Odczyt pliku nie powiódł się. | Sprawdź urządzenie i powtórz wywołanie. |
| `E_CANCELLED` | Wywołanie zostało anulowane albo osiągnęło limit czasu podczas haszowania. | Powtórz wywołanie z dłuższym `timeout`. |
| `E_TIMEOUT` | Wywołanie zostało anulowane albo osiągnęło limit czasu podczas haszowania. | Powtórz wywołanie z dłuższym `timeout`. |

## Powiązane {#related}

`info ... hash: sha256` zwraca skrót jednego pliku wraz z jego pozostałymi
szczegółami. `find`, `list` i `group` łączą się z `hash`, aby znajdować
zduplikowane pliki.
