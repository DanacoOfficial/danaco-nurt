---
command: archive.list
group: archive
summary: Wyświetla wpisy archiwum zip, tar, skompresowanego tar, 7z, jar, whl lub apk.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# archive.list

## Składnia {#synopsis}

```text
archive.list <path> [name: value]...
```

## Opis {#description}

`archive.list` tworzy jeden rekord `archive_entry` dla każdego składnika
archiwum `path`, w kolejności archiwum. Format jest rozpoznawany na podstawie
zawartości pliku, a nie jego nazwy: zip (w tym jar, whl i apk), tar, tar
skompresowany za pomocą gzip, zstd, xz, bzip2 lub lz4 oraz 7z.

Każdy rekord zawiera: archiwum, ścieżkę składnika z separatorami `/`, nazwę,
typ (`file`, `directory`, `link` lub `other`), rozmiar po rozpakowaniu,
rozmiar w archiwum, jeżeli format zapisuje go dla każdego składnika, czas
modyfikacji, uprawnienia w zapisie ósemkowym, cel dowiązania, metodę
kompresji, informację, czy zawartość jest zaszyfrowana, sumę kontrolną CRC-32,
jeżeli format ją zapisuje, oraz format archiwum. Ścieżki i nazwy są w postaci
NFC; nazwa zapisana bajtami, które nie są poprawnym UTF-8, jest pokazywana
z U+FFFD w miejscu takich bajtów.

`glob` wybiera składniki według ścieżki. Wzorzec bez `/` jest dopasowywany do
ostatniego elementu ścieżki składnika, wzorzec z `/` do całej ścieżki; `*` i `?`
nie dopasowują `/`, a `**` dopasowuje dowolną liczbę elementów.

Skompresowane archiwum tar i ciągłe archiwum 7z są dekodowane od początku, więc
ich składniki są zbierane przed utworzeniem pierwszego rekordu. Puste archiwum
nie tworzy żadnych rekordów.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik archiwum. |
| `glob` | `glob` | brak | Tylko wpisy, których ścieżka pasuje do tego wzorca. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `archive_entry` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie składników archiwum zip:

```nurt example
archive.list "/home/user/dl/site.zip"
```

```text expected
css           directory
css/site.css  file  4 B
index.html    file  13 B
```

Wyświetlenie wyłącznie źródeł Rust ze skompresowanego archiwum tar:

```nurt example
archive.list "/home/user/dl/src.tar.gz" glob: "*.rs"
```

Wyświetlenie archiwum tar skompresowanego za pomocą zstd, którego nazwa nie ma
rozszerzenia archiwum; pole `format` rekordów ma wartość `tar_zst`:

```nurt example
archive.list "/home/user/dl/backup.bin"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | `path` nie istnieje | Sprawdź ścieżkę. |
| `E_FORMAT` | plik nie jest archiwum obsługiwanego formatu albo archiwum jest uszkodzone | Sprawdź plik; dla skompresowanego pliku, który nie zawiera archiwum tar, użyj `decompress`. |
| `E_ENCRYPTED` | nagłówek archiwum 7z jest zaszyfrowany | Najpierw odszyfruj archiwum jego własnym narzędziem. |
| `E_INVALID_VALUE` | `glob` nie jest poprawnym wzorcem | Popraw wzorzec. |
| `E_LIMIT` | archiwum ma więcej składników niż limit rekordów | Wybierz mniej składników za pomocą `glob`. |
| `E_PATH_RELATIVE` | `path` jest względna i nie ustawiono katalogu bazowego | Podaj ścieżkę bezwzględną albo ustaw opcję `base`. |

## Powiązane {#related}

`archive.extract` rozpakowuje archiwum; `archive.create` je tworzy;
`decompress` dekompresuje pojedynczy skompresowany plik.
