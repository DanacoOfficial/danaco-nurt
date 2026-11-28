---
command: list
group: core
summary: Wyświetla wpisy katalogu.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# list

## Składnia {#synopsis}

```text
list <path> [name: value]...
```

## Opis {#description}

`list` produkuje jeden rekord `entry` dla każdego wpisu poniżej katalogu
`path`, do `depth` poziomów w głąb. Sam katalog nie jest rekordem; do
uzyskania rekordu pojedynczej ścieżki użyj `find` z `depth: 0`. Dowiązanie do
katalogu jest wyświetlane jako katalog, do którego prowadzi.

Każdy rekord zawiera `depth` (1 dla bezpośredniej zawartości) oraz
`relative`, czyli ścieżkę względną wobec `path` z separatorami `/`. Nazwy są w
postaci NFC; zapisana nazwa, która nie jest poprawnym Unicode albo nie jest w
NFC, jest zachowywana w `raw_name`. Nazwa, która nie jest poprawna w każdym
obsługiwanym systemie, zawiera `portability` z jedną lub kilkoma wartościami:
`windows_reserved_name`, `windows_invalid_character`,
`windows_trailing_character` i `invalid_unicode`.

Ścieżka, która nie jest poprawnym Unicode (w systemie Linux nazwa z bajtem
`0xFF`), jest zapisywana z U+FFFD w `path`, `parent` i `relative`; rekord
zawiera wtedy natywne bajty w `raw_path`, `raw_parent` i `raw_relative`, a cel
dowiązania w `raw_link_target`. Następny etap odczytuje wpis z `raw_path`,
więc `list ... |> remove` usuwa wpis, który został wyświetlony.

Kolejność zależy od `view`:

| `view` | Kolejność rekordów |
|---|---|
| `flat` | według `sort` dla wszystkich rekordów; remisy według ścieżki |
| `tree` | najpierw w głąb, każdy katalog przed swoją zawartością; wpisy jednego katalogu według `sort` |
| `sizes` | jak `tree`; każdy katalog zawiera `total_size` i `file_count` |

`sort: name` porównuje nazwy w NFC według punktów kodowych. `sort: type`
umieszcza najpierw katalogi, następnie pliki, dowiązania i inne wpisy.
`total_size` jest sumą rozmiarów wszystkich plików i dowiązań poniżej
katalogu, na dowolnej głębokości, a `file_count` jest liczbą tych plików; plik
z kilkoma dowiązaniami twardymi jest liczony raz. Przy `view: sizes` wartość
`sort: size` porządkuje katalogi według ich sum.

Na woluminie, który nie rozróżnia wielkości liter, `path` może różnić się
wielkością liter od zapisanej nazwy; wyświetlanie rozpoczyna się wtedy od
zapisanej pisowni, każde pole `path` ma tę pisownię, a uwaga `case_alias`
zapisuje podaną ścieżkę.

Pusty katalog nie produkuje rekordów.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Katalog do wyświetlenia. |
| `depth` | `int` | `1` | Liczba poziomów katalogów do wyświetlenia. |
| `type` | `enum(file\|directory\|link\|any)` | `any` | Rodzaj wyświetlanych wpisów. |
| `hidden` | `bool` | `true` | Uwzględnia wpisy ukryte. |
| `sort` | `enum(name\|modified\|size\|type)` | `name` | Klucz sortowania wpisów. |
| `order` | `enum(asc\|desc)` | `asc` | Kolejność sortowania. |
| `view` | `enum(flat\|tree\|sizes)` | `flat` | Lista płaska, drzewo albo drzewo z sumami rozmiarów katalogów. |

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

Uwagi: `permission_denied_skipped` dla katalogów, których nie można odczytać,
`source_unavailable` dla wpisów, których nie można odczytać z innej
przyczyny, `nfd_name` dla nazw zapisanych w innej postaci normalizacji,
`portability` dla nazw, które nie są przenośne, oraz `case_alias` dla
ścieżki podanej w innej wielkości liter. Każda uwaga podaje liczbę
dotyczących wpisów oraz próbkę do pięciu ścieżek.

## Przykłady {#examples}

Wyświetlenie wpisów katalogu posortowanych według nazwy:

```nurt example
list "/home/user/app"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/README.md", "type": "file", "content_utf8": "# app\n" },
  { "path": "/home/user/app/docs/guide.md", "type": "file", "content_utf8": "guide\n" },
  { "path": "/home/user/app/src/main.go", "type": "file", "content_utf8": "package main\n" },
  { "path": "/home/user/app/src/util/util.go", "type": "file", "content_utf8": "package util\n" }
] }
```

Wyświetlenie dwóch poziomów katalogu w postaci drzewa:

```nurt example
list "/home/user/app" depth: 2 view: tree
```

Wyświetlenie katalogów z łącznym rozmiarem ich zawartości, od największego:

```nurt example
list "/home/user/app" view: sizes sort: size order: desc
```

Wyświetlenie plików katalogu, od ostatnio zmienionego:

```nurt example
list "/home/user/app/src" type: file sort: modified order: desc
```

Odpowiedź, która pomija rekordy, kończy się wskazówką „more: next” i zawiera
pole `continuation`; polecenie `next` zwraca kolejną część bez ponownego
uruchamiania polecenia.

```text
matched 312  shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | `path` nie istnieje | Sprawdź ścieżkę albo wyświetl jej katalog nadrzędny. |
| `E_IO` | `path` nie jest katalogiem | Wyświetl katalog nadrzędny albo użyj `find` z `depth: 0`. |
| `E_PERMISSION` | `path` nie można odczytać | Wyświetl katalog, który konto może odczytać. |
| `E_PATH_RELATIVE` | `path` jest względna i nie ustawiono katalogu bazowego | Podaj ścieżkę bezwzględną albo ustaw opcję `base`. |
| `E_INVALID_VALUE` | `depth` jest mniejsze niż 1 | Podaj głębokość 1 lub większą. |
| `E_UNKNOWN_VALUE` | wartość `type`, `sort`, `order` lub `view` jest nieznana | Użyj jednej z wymienionych wartości. |
| `E_LIMIT` | wyświetlanie przekracza limit rekordów | Zmniejsz `depth` albo użyj `find` z `glob` lub `exclude`. |

## Powiązane {#related}

`find` wybiera wpisy według wzorca nazwy i atrybutów na dowolnej głębokości;
`info` opisuje jeden wpis. `help skill: "files"` opisuje pracę z plikami.
