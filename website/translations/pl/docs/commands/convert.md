---
command: convert
group: text
summary: Konwertuje kodowanie tekstu, zakończenia wierszy lub znacznik kolejności bajtów plików.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# convert

## Składnia {#synopsis}

```text
convert <path>... [name: value]...
```

## Opis {#description}

`convert` przepisuje pliki tekstowe w innym kodowaniu, z innymi zakończeniami
wierszy albo z dodanym lub usuniętym znacznikiem kolejności bajtów (BOM).
Należy podać co najmniej jeden z parametrów `encoding`, `line_ending` i `bom`.

Kodowanie i zakończenia wierszy każdego pliku są wykrywane tak jak w `read`;
kodowania inne niż UTF-8 i UTF-16 są rozpoznawane na podstawie listy z klucza
konfiguracji `text.legacy_encodings`. Cała konwersja jest sprawdzana przed
zmianą jakiegokolwiek pliku: znak, którego kodowanie docelowe nie potrafi
przedstawić, kończy się błędem `E_ENCODING` ze ścieżką, wierszem i kolumną
znaku, a żaden plik nie jest zapisywany. Plik, który ma już żądaną postać, nie
jest zapisywany i jest zgłaszany jako `unchanged`.

`line_ending` konwertuje każde zakończenie wiersza, także w plikach z mieszanymi
zakończeniami; brakujące zakończenie wiersza na końcu pliku pozostaje
brakujące. Bez `bom` plik, który zachowuje kodowanie, zachowuje swój znacznik
kolejności bajtów, plik skonwertowany do UTF-16 go otrzymuje, a plik
skonwertowany do dowolnego innego kodowania jest zapisywany bez niego. Znacznik
kolejności bajtów istnieje tylko w UTF-8 i UTF-16.

W postaci źródła każda ścieżka musi wskazywać plik tekstowy. W postaci etapu
wejściowe rekordy `entry` wskazują pliki; rekordy katalogów i innych wpisów,
które nie są zwykłymi plikami, są pomijane, a pliki o zawartości binarnej są
pomijane i liczone w `binary_skipped`. Ścieżka podana dwa razy jest
konwertowana jeden raz.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `list<path>` | brak | Pliki do skonwertowania w postaci źródła. |
| `encoding` | `text` | brak | Kodowanie docelowe, na przykład utf-8 lub windows-1250. |
| `line_ending` | `enum(lf\|crlf\|cr\|keep)` | `keep` | Docelowe zakończenie wiersza albo keep, aby pozostawić zakończenia wierszy bez zmian. |
| `bom` | `bool` | brak | Dodaje lub usuwa znacznik kolejności bajtów. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none`, `entry` | `effect` |

Rekord `effect` ma jedną zmianę `change` na każdy skonwertowany plik z polami
`encoding_before`, `encoding_after`, `line_ending_before`, `line_ending_after`,
`bom_before`, `bom_after`, `size_before` i `size_after`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

Wpisy szczególne: `encoding_changed` i `line_ending_changed` z wartościami
przed zmianą i po niej w `detail`, `bom_changed` z `added` lub `removed`,
`unchanged` dla plików już w żądanej postaci, `binary_skipped` oraz
`nothing_matched` dla pustego wejścia.

## Przykłady {#examples}

Konwersja pliku Windows-1250 z zakończeniami wierszy CRLF do UTF-8 z LF.

```nurt example
convert "/srv/legacy/notes.txt" encoding: "utf-8" line_ending: lf
```

```text expected
op 31  convert  ok
targets 1 file
  ~ /srv/legacy/notes.txt  windows-1250 -> utf-8, crlf -> lf
notable
  i encoding_changed  /srv/legacy/notes.txt  windows-1250 -> utf-8
  i line_ending_changed  /srv/legacy/notes.txt  crlf -> lf
```

Konwersja zakończeń wierszy wymienionych skryptów na CRLF.

```nurt example
find "/srv/app/scripts" glob: "*.cmd" type: file |> convert line_ending: crlf
```

```text expected
op 32  convert  ok
targets 1 file
  ~ /srv/app/scripts/build.cmd  lf -> crlf
notable
  i line_ending_changed  /srv/app/scripts/build.cmd  lf -> crlf
  i unchanged  /srv/app/scripts/test.cmd
```

Usunięcie znacznika kolejności bajtów z pliku CSV.

```nurt example
convert "/srv/app/data.csv" bom: false
```

```text expected
op 33  convert  ok
targets 1 file
  ~ /srv/app/data.csv  bom removed
notable
  i bom_changed  /srv/app/data.csv  removed
```

Konwersja pliku w stronie kodowej DOS 852, z `ibm852` w
`text.legacy_encodings`, do UTF-8.

```nurt example
convert "/srv/dos/menu.txt" encoding: "utf-8"
```

Znak, którego Latin-2 nie potrafi przedstawić, zatrzymuje konwersję.

```nurt example
convert "/srv/app/notes.txt" encoding: "iso-8859-2"
```

```text expected
error E_ENCODING  stage 4  /srv/app/notes.txt:2:8
  character `→` (U+2192) at line 2, column 8 cannot be represented in `iso-8859-2`
  fix: convert "/srv/app/notes.txt" encoding: "utf-8"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Brak `path` w postaci źródła albo brak każdego z `encoding`, `line_ending` i `bom`. | Podaj pliki i postać, do której mają zostać skonwertowane. |
| `E_PARAM_CONFLICT` | `path` podano w postaci etapu. | Usuń `path` albo rozpocznij potok od `convert`. |
| `E_UNKNOWN_VALUE` | `encoding` lub `line_ending` ma nieznaną wartość. | Użyj wymienionego kodowania lub zakończenia wiersza. |
| `E_INVALID_VALUE` | `bom: true` dla kodowania bez znacznika kolejności bajtów (`problem: bom_not_supported`) albo ścieżka, która nie jest zwykłym plikiem (`problem: not_a_file`). | Usuń `bom` albo skonwertuj pliki katalogu za pomocą `find ... type: file \|> convert`. |
| `E_ENCODING` | Plik jest niepoprawny w wykrytym kodowaniu albo znaku nie można przedstawić w kodowaniu docelowym; `details` podają `character`, `line`, `column` i `byte`. | Skonwertuj do `utf-8` albo usuń znak. |
| `E_BINARY` | Ścieżka postaci źródła ma zawartość binarną. | Podawaj wyłącznie pliki tekstowe. |
| `E_LIMIT` | Plik jest większy niż limit plików tekstowych. | Konwertuj mniejsze pliki. |
| `E_STALE` | Plik zmienił się po sporządzeniu planu. | Powtórz wywołanie. |
| `E_NOT_FOUND` | Pliku nie można odczytać ani zapisać. | Sprawdź ścieżkę i jej uprawnienia. |
| `E_PERMISSION` | Pliku nie można odczytać ani zapisać. | Sprawdź ścieżkę i jej uprawnienia. |
| `E_IO` | Pliku nie można odczytać ani zapisać. | Sprawdź ścieżkę i jej uprawnienia. |

## Powiązane {#related}

`read ... encoding:` odczytuje plik w podanym kodowaniu. `write`, `replace`,
`insert` i `edit` zachowują kodowanie i zakończenia wierszy pliku; `convert`
jest jawnym sposobem ich zmiany. `undo` przywraca poprzednią zawartość.
