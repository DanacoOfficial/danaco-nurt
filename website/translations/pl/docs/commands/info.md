---
command: info
group: core
summary: Opisuje ścieżki, także ścieżki, które nie istnieją.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# info

## Składnia {#synopsis}

```text
info <path>... [name: value]...
```

## Opis {#description}

`info` tworzy jeden rekord `entry` na ścieżkę, w podanej kolejności. Ścieżka,
która nie istnieje albo której element nadrzędny nie jest katalogiem, daje wpis
z `exists: false`, `size` równym zero i epoką Unix jako `modified`; nie jest to
błąd.

Istniejący wpis zawiera pola ścieżki i nazwy, `type`, `size`, czasy, `mode` w
zapisie ósemkowym, `readonly`, `hidden`, `links`, `etag`, konto (`owner`) i
grupę pliku, `acl`, `xattrs`, `streams`, `volume` oraz `file_id`. Dowiązanie
jest opisywane bez podążania za nim; `link_target` zawiera cel w postaci
zapisanej w dowiązaniu, a `real_path` ścieżkę po rozwiązaniu dowiązań, gdy cel
istnieje.

Zwykły plik, a także dowiązanie, które do niego prowadzi, zawiera również
`mime`, `binary`, `encoding`, `line_ending` i `bom`, rozpoznane z pierwszych
8192 bajtów zawartości. Typ nośnika pochodzi z sygnatury zawartości, a dla
tekstu następnie z nazwy pliku; w pozostałych przypadkach jest to `text/plain`
lub `application/octet-stream`.

`lines`, `hash` i grupy `details` odczytują więcej zawartości i są obliczane
tylko na żądanie. Pliki większe niż limit tekstu wykonania nie mają liczby
wierszy ani szczegółów `text`, `pdf` i `symbols`.

Wpis, którego znacznik encji jest równy znacznikowi podanemu dla jego ścieżki w
`known`, jest raportowany tylko ze ścieżką, nazwą, typem, rozmiarem, czasami,
trybem i `etag`, a istotny wpis `unchanged` wskazuje ścieżkę.

Bez ścieżki wynik jest pusty.

`owner` i `group` zawierają nazwę i identyfikator liczbowy, jak w
`user (1000)`, w systemach Linux i macOS; nazwy pochodzą z tablic kont systemu,
a konto bez wpisu w nich jest pokazywane przez identyfikator. W systemie Windows
`owner` zawiera identyfikator zabezpieczeń konta, a `group` jest nieobecne.

`acl` wyświetla POSIX ACL w skróconej postaci `getfacl` w systemie Linux (na
przykład `user:alice:r--`) oraz wpisy listy kontroli dostępu w systemie macOS; w
systemie Windows jest nieobecne. Wartości atrybutów rozszerzonych, które nie są
tekstem UTF-8, są zapisywane jako `"hex:"`, po którym następują cyfry
szesnastkowe.

`created` jest obecne tam, gdzie system raportuje czas utworzenia.

Grupy `git` i `symbols` wymagają usług repozytorium i kodu silnika. Bez usługi
grupa jest pomijana, a istotny wpis `source_unavailable` wskazuje brakującą
usługę raz na wywołanie; plik w języku bez gramatyki nie ma grupy `symbols`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `list<path>` | wymagany | Ścieżki do opisania; brakująca ścieżka daje exists: false. |
| `hash` | `enum(none\|sha256\|sha512\|sha1\|md5\|blake3\|crc32)` | `none` | Algorytm skrótu zawartości. |
| `details` | `list<enum(text\|exif\|pdf\|media\|git\|symbols)>` | `[]` | Dodatkowe grupy metadanych. |
| `known` | `map<text>` | brak | Mapa ze ścieżki na etag; niezmienione ścieżki są raportowane bez szczegółów. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `entry`, `registry_value` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Opisz jeden plik:

```nurt example
info "/home/user/notes.txt"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes.txt", "type": "file", "content_utf8": "line one\nline two\n" }
] }
```

Opisz istniejącą i brakującą ścieżkę; drugi wpis ma
`exists: false`:

```nurt example
info "/home/user/notes.txt" "/home/user/draft.txt"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes.txt", "type": "file", "content_utf8": "line one\nline two\n" }
] }
```

Dodaj skrót SHA-256 i liczbę wierszy:

```nurt example
info "/home/user/notes.txt" hash: sha256 details: [text]
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes.txt", "type": "file", "content_utf8": "line one\nline two\n" }
] }
```

Opisz dowiązanie; `link_target` i `real_path` wskazują plik, na który ono
prowadzi:

```nurt example
info "/home/user/current"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes.txt", "type": "file", "content_utf8": "line one\nline two\n" },
  { "path": "/home/user/current", "type": "symlink", "target": "/home/user/notes.txt" }
] }
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_PATH_RELATIVE` | ścieżka względna bez katalogu bazowego | podaj ścieżkę bezwzględną albo ustaw opcję `base` |
| `E_UNKNOWN_VALUE` | nieznany algorytm skrótu lub grupa `details` | użyj wartości wymienionej w tabeli parametrów |
| `E_INVALID_VALUE` | wartość `known` nie jest etagiem | przekaż pole `etag` wpisu odczytanego wcześniej |
| `E_TYPE` | parametr ma wartość innego typu | przekaż listę nazw do `details` i mapę do `known` |
| `E_PERMISSION` | nie można odczytać metadanych ani zawartości wpisu | sprawdź uprawnienia wpisu i jego elementów nadrzędnych |
| `E_IO` | odczyt systemu plików nie powiódł się | powtórz wywołanie; sprawdź wolumin |

## Powiązane {#related}

`list`, `find`, `read`, `hash`, `attributes`.
