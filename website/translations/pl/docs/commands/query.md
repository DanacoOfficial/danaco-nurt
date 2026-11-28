---
command: query
group: documents
summary: Wybiera i przekształca wartości dokumentów za pomocą filtra jq.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# query

## Składnia {#synopsis}

```text
content|entry|item|response |> query <expression>
query <expression> path: <path>
```

## Opis {#description}

`query` stosuje filtr w składni `jq` do dokumentów i tworzy jeden rekord `item` dla każdego wyniku. Filtry są kompilowane i wykonywane w tym samym procesie; program `jq` nie jest potrzebny.

W postaci źródłowej dokumentem jest plik `path`. W postaci etapu każdy rekord wejściowy dostarcza wartość przekazywaną do filtra:

| Wejście | Wartość przekazywana do filtra |
|---|---|
| `content` | tekst odczytany jako dokument swojego formatu; JSON, gdy żaden format nie zostanie rozpoznany |
| `entry` | plik wpisu odczytany jako dokument, z `raw_path`, gdy wpis go ma (nazwa niebędąca poprawnym Unicode); wpisy inne niż pliki są odrzucane |
| `item` | wartość elementu; klucze wyników rozszerzają klucz elementu |
| `response` | treść odczytana jako dokument; JSON, gdy typ zawartości wskazuje JSON lub żaden format nie zostanie rozpoznany |

Każdy obsługiwany format dokumentu jest odczytywany do tego samego modelu wartości: map, list, tekstu, liczb, wartości logicznych i null. Daty i godziny są tekstem w postaci RFC 3339, dane binarne są tekstem Base64, a liczby zachowują dokładne cyfry, gdy postać binarna by je zmieniła. Wielodokumentowy strumień YAML daje jedną wartość dla każdego dokumentu.

Wynik wybrany przez filtr za pomocą ścieżki, takiej jak `.spec.replicas` lub `.items[]`, otrzymuje jako `key` kanoniczną ścieżkę klucza swojej pozycji w dokumencie, tak aby `set` i `remove` mogły go zmienić. Pozostałe wyniki, takie jak `length` lub obiekt zbudowany przez filtr, otrzymują klucz `"."`. Filtr tożsamościowy dla całego dokumentu daje klucz `$`. `value_type` jest typem wartości wyniku; `path`, `format` i `etag` pochodzą z wejścia.

Filtr podany jako stała jest kompilowany przy wiązaniu programu, przed odczytem jakiegokolwiek dokumentu. Filtr, który się nie kompiluje, kończy się błędem `E_QUERY` z wierszem i kolumną problemu w `details`. Błąd filtra w czasie wykonania, w tym `error(…)`, kończy się błędem `E_QUERY` po wynikach utworzonych wcześniej, z plikiem w `at`. `halt` kończy filtr; `halt_error` kończy się błędem `E_QUERY` z tekstem zapisanym przez filtr.

`env` i `$ENV` odczytują środowisko sesji, `now` odczytuje zegar sesji, a `debug` i `stderr` zapisują zdarzenia dziennika. `input`, `inputs` i moduły nie są dostępne.

Język filtrów jest zgodny z `jq` 1.8.1. Mnożenie ciągu znaków przez liczbę, usunięcie kilku ścieżek, aktualizacja poniżej `null` lub za końcem listy, ułamkowy indeks listy, indeksowanie obiektu liczbą, wycinek `null`, `@base64d`, `repeat` i `fromstream` dają wyniki `jq` 1.8.1. Dwa wyniki się od niego różnią:

| Przypadek | `query` | `jq` 1.8.1 |
|---|---|---|
| `captures` z `match` | grupy, które nie brały udziału, są pomijane | grupy, które nie brały udziału, mają wartość `null` |
| `@base32`, `@base32d` | filtr się nie kompiluje | błąd czasu wykonania |

Aktualizacja rozszerzająca listę oraz ciąg znaków utworzony przez mnożenie ciągu mają rozmiar ograniczony przez `limits.max_file_bytes_text`; większy wynik kończy się błędem `E_LIMIT`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `expression` | `text` | wymagany | Filtr w składni jq. |
| `path` | `path` | none | Dokument do zapytania w postaci źródłowej. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| `stage` | `content`, `entry`, `item`, `response` | `item` |
| `source` | `none` | `item` |

Wyniki są tworzone w trakcie działania filtra. Potok, który używa tylko pierwszych wyników, zatrzymuje filtr.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `transform` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie nazw zależności manifestu pakietu:

```nurt example
query ".dependencies | keys[]" path: "/home/user/app/package.json"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/package.json", "type": "file", "content_utf8": "{\n  \"name\": \"app\",\n  \"dependencies\": {\n    \"react\": \"^19.0.0\",\n    \"zod\": \"^3.24.0\"\n  }\n}\n" }
] }
```

Wybór obrazów z pliku YAML odczytanego poleceniem `read`:

```nurt example
read "/home/user/app/compose.yaml" |> query ".services[].image"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/compose.yaml", "type": "file", "content_utf8": "services:\n  web:\n    image: nginx:1.27\n  db:\n    image: postgres:17\n" }
] }
```

```text expected
nginx:1.27
```

Zwiększenie każdej liczby replik o jeden za pomocą kluczy wyników:

```nurt example
query ".deployments[] | select(.replicas < 3) | .replicas" path: "/home/user/app/scale.json" |> set (value + 1)
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/scale.json", "type": "file", "content_utf8": "{\"deployments\": [{\"name\": \"api\", \"replicas\": 2}, {\"name\": \"worker\", \"replicas\": 5}]}\n" }
] }
```

Policzenie sekcji pliku TOML:

```nurt example
query "[.[] | objects] | length" path: "/home/user/app/config.toml"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/config.toml", "type": "file", "content_utf8": "[server]\nport = 8080\n\n[database]\nurl = \"postgres://localhost/app\"\n" }
] }
```

Odpowiedź, która pomija rekordy, kończy się wskazówką „more: next” i zawiera pole `continuation`; polecenie `next` zwraca kolejną część bez ponownego uruchamiania polecenia.

```text
matched 700  shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_QUERY` | filtr się nie kompiluje (`problem`: `syntax`, `undefined`, `module`), kończy się błędem w czasie działania (`runtime`) lub kończy się przez `halt_error` (`halt`) | popraw filtr; `details` zawiera wiersz i kolumnę błędu kompilacji |
| `E_DOC_PARSE` | dokument nie jest poprawny w swoim formacie albo tekst bez rozpoznanego formatu nie jest JSON | popraw dokument lub odczytaj go w odpowiednim formacie |
| `E_NOT_FOUND` | plik nie istnieje | sprawdź ścieżkę |
| `E_IO` | pliku nie można odczytać | zobacz komunikat |
| `E_BINARY` | rekord zawartości zawiera dane binarne | wykonuj zapytania tylko na dokumentach tekstowych |
| `E_TYPE` | rekord wejściowy jest innego rodzaju lub wpis nie jest plikiem | przekaż rekordy `content`, `entry`, `item` lub `response` |
| `E_MISSING_PARAM` | postać źródłowa nie ma `path` | dodaj `path:` |
| `E_STALE` | plik zmienił się w trakcie odczytu | uruchom polecenie ponownie |
| `E_LIMIT` | filtr tworzy więcej wyników, niż może pomieścić jedno wykonanie, mnoży ciąg znaków do wyniku dłuższego niż `limits.max_file_bytes_text` bajtów (`"x" * n`) albo plik jest większy niż limit rozmiaru dokumentu | zawęź filtr, użyj `limit(n; f)` lub podnieś limit |
| `E_CANCELLED` | wykonanie zostało anulowane lub upłynął jego limit czasu, także wewnątrz nieskończonych generatorów, takich jak `repeat` | uruchom polecenie ponownie z węższym filtrem |
| `E_TIMEOUT` | wykonanie zostało anulowane lub upłynął jego limit czasu, także wewnątrz nieskończonych generatorów, takich jak `repeat` | uruchom polecenie ponownie z węższym filtrem |

## Powiązane {#related}

`read` z `key:` dla wartości pod ścieżkami kluczy, `set` do zmiany wartości znalezionych przez `query`, `sql` dla baz danych SQLite.
