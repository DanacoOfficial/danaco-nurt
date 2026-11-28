---
command: replace
group: core
summary: Zastępuje tekst w plikach lub w znalezionym tekście.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# replace

## Składnia {#synopsis}

```text
replace <pattern> <with> in: <list<path>> [name: value]...
match|node|item |> replace <with> [name: value]...
line|content |> replace <pattern> <with> [name: value]...
```

## Opis {#description}

`replace` zmienia tekst w plikach. Postać źródłowa zastępuje dopasowania `pattern` w plikach wymienionych w `in`; katalog z `in` wnosi swoje pliki pasujące do `glob` i `file_type`. Postać dla rekordów `line` i `content` zastępuje dopasowania `pattern` wewnątrz wierszy rekordów: rekord `line` wybiera swój wiersz, rekord `content` swój `range` lub cały plik. Postać dla rekordów `match` zastępuje tekst każdego dopasowania wejściowego, na przykład wyniku `search`.

Wzorzec tekstowy jest dosłowny. Jest dopasowywany z tolerancją wskazaną przez `whitespace`: z `normalize`, wartością domyślną, tolerowane są różnice wcięć, białych znaków na końcu wiersza, zakończeń wierszy i ciągów białych znaków, dokładne wystąpienie zawsze ma pierwszeństwo, a zastępnik przejmuje wcięcie, jednostkę wcięcia i zakończenia wierszy miejsca, które zastępuje. Wyrażenie regularne `/…/` jest dopasowywane dokładnie. `^` i `$` dopasowują się na granicach wierszy, a dopasowanie nie obejmuje wielu wierszy, chyba że ustawiono `multiline`, co pozwala też, aby `.` dopasowywało koniec wiersza. Wyrażenie regularne z wyszukiwaniem w przód lub wstecz albo z odwołaniami wstecznymi jest obliczane przez silnik z nawrotami.

Dla wyrażeń regularnych i dopasowań wejściowych `with` jest szablonem: `$1` i `${1}` wstawiają grupę numerowaną, `${name}` grupę nazwaną, `$0` całe dopasowanie, a `$$` znak `$`. Każdy inny `$` jest błędem. Z `template: false` tekst jest wstawiany tak, jak zapisano. Dla dopasowań wejściowych `$n` to `groups[n]` rekordu (`groups[0]` to całe dopasowanie, tak jak zapisuje je `search`), a grupy nazwane to jego mapa `named`. Wzorzec dosłowny zawsze wstawia `with` tak, jak zapisano.

`expect` zlicza dopasowania wszystkich plików, zanim zastosowane zostanie `occurrence`; `occurrence` zachowuje jedno dopasowanie, liczone od 1, w każdym pliku. Gdy liczba jest poza `expect`, nic nie jest zmieniane, a błąd wyświetla pierwsze dopasowania. Gdy nic nie pasuje, `on_missing: skip` zgłasza godny uwagi skutek `nothing_matched` i niczego nie zmienia, a `on_missing: error` kończy się błędem z najbliższymi fragmentami tekstu. Gdy wzorzec pasuje, ale żaden plik nie ma wskazanego wystąpienia, `on_missing: error` kończy się błędem `E_OUT_OF_RANGE` i wyświetla pierwsze dopasowania.

`scope` zachowuje dopasowania zaczynające się w kodzie, komentarzach lub literałach tekstowych, zgodnie z klasyfikacją według gramatyki pliku. `mode: syntax` dopasowuje wzorzec strukturalny zapisany w składni języka pliku, a `with` jest szablonem strukturalnym. Obie opcje wymagają gramatyki dla pliku.

Z `check_references` pliki są po zmianie przeszukiwane pod kątem zastąpionego tekstu poza zastępnikami: symbolicznie, gdy gramatyka zna plik, a tekst jest identyfikatorem, w przeciwnym razie jako całe słowo. Znalezione miejsca są zgłaszane jako godny uwagi skutek `references_left` z ich liczbą i pierwszymi miejscami; nie uniemożliwiają zmiany.

Każdy plik zachowuje swoje kodowanie, znacznik kolejności bajtów i zakończenia wierszy. Plik, którego kodowanie nie może przedstawić nowego tekstu, nie jest zmieniany. Pliki binarne znalezione w katalogu są pomijane i liczone jako `binary_skipped`; plik binarny wymieniony w `in` jest pomijany jako `binary_target`. Z `binary: text` plik binarny jest odczytywany jako tekst Windows-1252.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `pattern` | `text \| regex` | wymagany | Tekst lub wyrażenie regularne do zastąpienia. |
| `with` | `text` | wymagany | Zastępnik; $1, ${name} i $$ odnoszą się do grup, chyba że template ma wartość false. |
| `in` | `list<path>` | wymagany | Pliki lub katalogi w postaci źródłowej. |
| `glob` | `glob \| list<glob>` | `"**"` | Wzorce nazw plików w katalogach. |
| `mode` | `enum(auto\|literal\|regex\|syntax)` | `auto` | Tryb wzorca; auto traktuje tekst jako dosłowny, a /…/ jako wyrażenie regularne. |
| `occurrence` | `int` | każde wystąpienie | Zastępuje tylko to wystąpienie, liczone od 1. |
| `case_sensitive` | `bool` | `true` | Dopasowuje z uwzględnieniem wielkości liter. |
| `whole_word` | `bool` | `false` | Dopasowuje tylko całe słowa. |
| `multiline` | `bool` | `false` | Dopuszcza dopasowania obejmujące wiele wierszy. |
| `whitespace` | `enum(exact\|trailing\|indent\|normalize)` | `normalize` | Tolerancja różnic białych znaków; normalize dostosowuje też zastępnik do stylu pliku. |
| `scope` | `enum(any\|code\|comments\|strings)` | `any` | Część kodu źródłowego, w której dopasowania są zastępowane. |
| `check_references` | `bool` | `false` | Zgłasza odwołania do zastąpionego tekstu pozostałe po zmianie. |
| `template` | `bool` | `true` | Traktuje zastępnik jako szablon z odwołaniami do grup. |
| `on_missing` | `enum(skip\|error)` | `skip` | Zachowanie, gdy nic nie pasuje. |
| `expect` | `range` | none | Oczekiwana liczba zastąpień; w przeciwnym razie nic nie jest zmieniane. |
| `if_match` | `text \| map<text>` | none | Oczekiwany etag pliku lub mapa ze ścieżki na etag dla kilku plików. |
| `validate` | `enum(off\|warn\|reject)` | `reject` | Wycofuje zmianę, która dodaje nowy błąd składni do znanego formatu. |
| `hidden` | `bool` | `true` | Uwzględnia pliki ukryte. |
| `gitignore` | `bool` | `false` | Stosuje reguły plików .gitignore. |
| `binary` | `enum(skip\|text)` | `skip` | Pomija pliki binarne lub traktuje je jako tekst. |
| `file_type` | `list<text>` | `[]` | Typy plików według rozszerzenia, takie jak go lub ts. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| `source` | `none` | `effect` |
| `matches` | `match`, `node`, `item` | `effect` |
| `lines` | `line`, `content` | `effect` |

Rekord `effect` wyświetla jedną zmianę `change` dla każdego zmienionego pliku, z liczbą zastąpień, dodanych i usuniętych wierszy oraz różnicą.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Zastąpienie pierwszego wywołania w jednym pliku:

```nurt example
replace "oldName(" "newName(" in: ["/home/user/app/a.go"] occurrence: 1
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/a.go", "type": "file", "content_utf8": "x := oldName(1)\ny := oldName(2)\n" }
] }
```

Przepisanie wywołań za pomocą wyrażenia regularnego w plikach Rust katalogu:

```nurt example
replace /(\w+\([^)]*\))\.unwrap\(\)/ "$1?" in: ["/home/user/app"] glob: "**/*.rs"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/src/main.rs", "type": "file", "content_utf8": "let a = load().unwrap();\n" },
  { "path": "/home/user/app/src/io/read.rs", "type": "file", "content_utf8": "let b = open(p).unwrap();\n" },
  { "path": "/home/user/app/README.md", "type": "file", "content_utf8": "call().unwrap()\n" }
] }
```

Zastąpienie bloku zapisanego z innym wcięciem; nowe wiersze przejmują wcięcie pliku:

```nurt example
replace "start()\nreport()" "prepare()\nstart()\nreport()" in: ["/home/user/app/run.py"]
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/run.py", "type": "file", "content_utf8": "def run():\n    if ready:\n        start()\n        report()\n" }
] }
```

Zastąpienie słowa tylko w wierszach 2 i 3:

```nurt example
lines "/home/user/notes.txt" 2..3 |> replace "draft" "final"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes.txt", "type": "file", "content_utf8": "draft one\ndraft two\ndraft three\n" }
] }
```

Przeformatowanie dopasowań wyszukiwania wraz z ich grupami:

```nurt example
search /(\w+)=(\S+)/ "/home/user/app/config.ini" |> replace "$1 = $2"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/config.ini", "type": "file", "content_utf8": "port=8080\nhost=example.com\n" }
] }
```

Zastąpienie schematu tylko wtedy, gdy występuje w katalogu dokładnie trzy razy:

```nurt example
replace "http://" "https://" in: ["/home/user/site"] expect: 3
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/site/a.html", "type": "file", "content_utf8": "http://example.com http://example.com\n" },
  { "path": "/home/user/site/b.html", "type": "file", "content_utf8": "http://example.com\n" }
] }
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_ASSERT` | liczba dopasowań jest poza `expect` | rozszerz wzorzec, ustaw `occurrence` lub zmień `expect` |
| `E_NO_MATCH` | nic nie pasuje, a `on_missing` ma wartość `error` | odczytaj bieżący tekst i podaj istniejący fragment |
| `E_OUT_OF_RANGE` | żaden plik nie ma wskazanego wystąpienia, a `on_missing` ma wartość `error` | podaj `occurrence` od 1 do liczby dopasowań |
| `E_REGEX` | wyrażenie regularne lub szablon jest niepoprawne | popraw wzorzec lub przekaż `template: false` |
| `E_INVALID_VALUE` | wzorzec jest pusty albo `occurrence` jest mniejsze niż 1 | podaj tekst do zastąpienia i wystąpienie od 1 |
| `E_STALE` | plik zmienił się od odczytu lub nie ma znacznika `if_match` | odczytaj plik ponownie i powtórz zmianę |
| `E_ENCODING` | kodowanie pliku nie może przedstawić zastępnika | najpierw przekonwertuj plik poleceniem `convert` |
| `E_BINARY` | plik wskazany przez rekord wejściowy jest binarny | przekaż tylko pliki tekstowe |
| `E_UNSUPPORTED_LANGUAGE` | `scope` lub `mode: syntax` nie ma gramatyki dla pliku | użyj `scope: any` lub wzorca dosłownego |
| `E_LIMIT` | wyrażenie regularne przekroczyło limit nawrotów | uprość wzorzec |
| `E_ROLLED_BACK` | zmiana dodała błąd składni, a `validate` ma wartość `reject` | popraw nowy tekst lub przekaż `validate: warn` |

## Powiązane {#related}

`search`, `lines`, `edit`, `insert`, `write`, `diff`, `help skill: "text-editing"`.
