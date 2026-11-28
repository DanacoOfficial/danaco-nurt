---
command: search
group: core
summary: Przeszukuje zawartość plików w poszukiwaniu tekstu, wyrażeń regularnych lub wzorców składni.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# search

## Składnia {#synopsis}

```text
search <pattern> <path>... [name: value]...
entry|content|line |> search <pattern> [name: value]...
```

## Opis {#description}

`search` znajduje wzorzec w zawartości plików. Postać źródłowa przechodzi przez podane pliki i katalogi: katalog jest przeszukiwany rekurencyjnie, z filtrami `glob`, `exclude`, `exclude_preset`, `hidden` i `gitignore` oraz z typami plików z `file_type` i `exclude_file_type`. Postać etapu przeszukuje pliki wejściowych rekordów `entry` (wpis katalogu jest przeglądany jak w postaci źródłowej), tekst wejściowych rekordów `content` i tekst wejściowych rekordów `line`. Bez dopasowań wynik jest pusty.

`glob` i `exclude` porównują wzorce i ścieżki w postaci normalizacji Unicode C (NFC). Nazwa zapisana w postaci zdekomponowanej, jak na woluminach macOS, pasuje do wzorca zapisanego w postaci złożonej, a wzorzec zdekomponowany pasuje do nazwy złożonej.

Wzorzec to tekst, wyrażenie regularne zapisane jako `/…/flags` albo lista obu. Z `mode: auto` tekst jest dopasowywany dosłownie, a `/…/` jest wyrażeniem regularnym; `mode: literal` i `mode: regex` traktują każdy wzorzec w jeden sposób. Z listą wzorców każde dopasowanie zawiera `pattern`, indeks swojego wzorca od 0, a `output: counts` daje jedną liczbę dla każdego wzorca.

Wyrażenia regularne używają składni biblioteki `regex` języka Rust: klas takich jak `[a-z]`, `\d`, `\w`, `\s`, właściwości Unicode `\p{Greek}`, grup `( )`, `(?:…)`, grup nazwanych `(?<name>…)` i `(?P<name>…)` oraz flag `i`, `m`, `s` i `x`. Klasy, `\w`, `\b` i składanie wielkości liter zgodne są z Unicode. `^` i `$` dopasowują się na początku i końcu każdego wiersza. Wyszukiwanie w przód i wstecz oraz odwołania wsteczne są obliczane przez silnik z nawrotami z limitem kroków; takie dopasowania zgłaszają `regex_engine: backtracking`, a wiersz przekraczający limit kroków jest liczony jako niepasujący (godne uwagi `regex_backtracking`). W porównaniu z wyrażeniami regularnymi JavaScript `\d` i `\w` dopasowują także cyfry i litery spoza ASCII, a flagi `g`, `u` i `y` nie istnieją.

Każdy pasujący wiersz to jeden rekord `match` z pozycją jego pierwszego dopasowania. Z `only_matching` każde dopasowanie to jeden rekord, a `line` zawiera dopasowany tekst. `start` i `end` liczą bajty od początku pliku w postaci zapisanej, także w plikach, które nie są UTF-8; `column` liczy znaki od 1. Z `multiline` dopasowanie może przekraczać zakończenia wierszy i zgłasza `end_line` i `end_column`; wzorzec dosłowny zawierający zakończenie wiersza to implikuje.

Pliki binarne są pomijane i liczone (godne uwagi `binary_skipped`), chyba że podano `binary: text`. Tekst w innym kodowaniu, takim jak UTF-16 lub Windows-1250, jest dekodowany przed dopasowaniem. Wiersze dłuższe niż `max_line` znaków są skracane wokół pierwszego dopasowania, ze znacznikiem pominiętych znaków (godne uwagi `truncated_lines`).

`max_per_file` ogranicza liczbę dopasowań zgłaszanych dla jednego pliku; `0`, podobnie jak pominięcie, oznacza brak limitu.

`output: matches` daje rekordy `match`. `output: counts` daje jeden rekord `match` dla każdego pliku lub dla każdego pliku i wzorca, z `count`, czyli liczbą dopasowań, i pozycją pierwszego dopasowania. `output: files` i `output: unmatched` dają rekordy `entry` plików z dopasowaniami i bez nich, z `match_count`.

`sort: path` zachowuje kolejność przejścia (korzenie w podanej kolejności, następnie w głąb według nazwy) lub rekordów wejściowych i tworzy rekordy w miarę przeszukiwania plików. `sort: modified` umieszcza najpierw ostatnio zmodyfikowane pliki, `sort: count` pliki z największą liczbą dopasowań, a `sort: rank` pliki pasujące do największej liczby wzorców, następnie z największą liczbą dopasowań, następnie o najpłytszych ścieżkach; te kolejności najpierw przeszukują każdy plik. Z `adaptive: true` i limitem liczby rekordów (na przykład `|> first 50`) pliki z największą liczbą dopasowań są pokazywane w całości, dopóki rekordy mieszczą się w limicie, a pozostałe pliki są podawane jako jeden rekord `match` z `count` i `summarized: true`.

`scope`, `enclosing` i `mode: syntax` używają grupy poleceń code. `scope` zachowuje dopasowania w kodzie, komentarzach lub literałach tekstowych; plik języka bez gramatyki jest liczony wyłącznie jako kod. `enclosing` dodaje `container`, czyli kwalifikowaną nazwę, rodzaj i wiersze najbardziej wewnętrznej definicji wokół każdego dopasowania; bez grupy code pole jest pomijane. Z `mode: syntax` wzorzec jest wzorcem kodu języka ze zmiennymi meta, takimi jak `$X`; przechwycone zmienne meta znajdują się w `named`.

Pliki są przeszukiwane równolegle w granicach limitu wątków wykonania; kolejność wyniku nie zależy od liczby wątków.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `pattern` | `text \| regex \| list<text \| regex>` | wymagany | Wzorzec; z listą liczby są podawane dla każdego wzorca. |
| `path` | `list<path>` | wymagany | Pliki lub katalogi do przeszukania w postaci źródłowej. |
| `glob` | `glob \| list<glob>` | `"**"` | Wzorce nazw plików do przeszukania. |
| `mode` | `enum(auto\|literal\|regex\|syntax)` | `auto` | Tryb wzorca; auto traktuje tekst jako dosłowny, a /…/ jako wyrażenie regularne. |
| `case_sensitive` | `bool` | `true` | Dopasowuje z uwzględnieniem wielkości liter. |
| `whole_word` | `bool` | `false` | Dopasowuje tylko całe słowa. |
| `multiline` | `bool` | `false` | Dopuszcza dopasowania obejmujące wiele wierszy. |
| `invert` | `bool` | `false` | Tworzy wiersze, które nie pasują. |
| `only_matching` | `bool` | `false` | Podaje tylko dopasowany tekst zamiast całego wiersza. |
| `trim` | `bool` | `false` | Usuwa początkowe i końcowe białe znaki wierszy. |
| `dedupe` | `bool` | `false` | Odrzuca powtórzone identyczne wiersze. |
| `scope` | `enum(any\|code\|comments\|strings)` | `any` | Część kodu źródłowego, w której dopasowania się liczą. |
| `enclosing` | `bool` | `false` | Dodaje obejmujący element kodu każdego dopasowania. |
| `max_line` | `int` | `500` | Maksymalna długość zgłaszanego wiersza w znakach. |
| `exclude` | `list<glob>` | `[]` | Wzorce do wykluczenia, w składni gitignore. |
| `exclude_preset` | `list<enum(vcs\|deps\|build\|cache)>` | `[]` | Predefiniowane wykluczenia: kontrola wersji, zależności, wynik kompilacji, pamięci podręczne. |
| `context` | `int` | `0` | Liczba wierszy kontekstu przed każdym dopasowaniem i po nim. |
| `before` | `int` | none | Liczba wierszy kontekstu przed każdym dopasowaniem. |
| `after` | `int` | none | Liczba wierszy kontekstu po każdym dopasowaniu. |
| `output` | `enum(matches\|files\|counts\|unmatched)` | `matches` | Dopasowania, pasujące pliki, liczby dla każdego pliku lub pliki bez dopasowań. |
| `max_per_file` | `int` | none | Maksymalna liczba dopasowań na plik; `0` i brak wartości oznaczają brak limitu. |
| `binary` | `enum(skip\|text)` | `skip` | Pomija pliki binarne lub przeszukuje je jako tekst. |
| `whitespace` | `enum(exact\|trailing\|indent\|normalize)` | `exact` | Tolerancja różnic białych znaków dla wzorców dosłownych. |
| `hidden` | `bool` | `true` | Uwzględnia pliki ukryte. |
| `gitignore` | `bool` | `false` | Stosuje reguły plików .gitignore. |
| `language` | `text` | none | Język programowania wzorców składni. |
| `file_type` | `list<text>` | `[]` | Typy plików według rozszerzenia, takie jak go lub ts; obowiązuje w każdym trybie wyjścia. |
| `exclude_file_type` | `list<text>` | `[]` | Typy plików według rozszerzenia do pominięcia. |
| `sort` | `enum(path\|modified\|count\|rank)` | `path` | Klucz sortowania wyników; rank porządkuje według trafności. |
| `adaptive` | `bool` | `false` | Dopasowuje wynik do limitu: pełne dopasowania dla plików z największą liczbą dopasowań, liczby dla pozostałych. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| `source` | `none` | `match`, `entry` |
| `stage` | `entry`, `content`, `line` | `match`, `entry` |

Pola `match`: `path`, `no`, `column`, `start`, `end`, `text`, `line`, `etag` oraz, gdy mają zastosowanie, `groups` (grupa 0 to całe dopasowanie), `named`, `before`, `after`, `count`, `pattern`, `container`, `regex_engine`, `end_line`, `end_column`, `summarized`. W postaci etapu przesunięcia rekordu `content` z zakresem wierszy oraz rekordu `line` bez `start` są liczone od początku tekstu rekordu; kolejne rekordy `line` jednego pliku są liczone łącznie.

Postać tekstowa wyniku pokazuje kolumny `path`, `no`, `column`, `text` oraz, gdy ma zastosowanie, `count`. Wszystkie pola pozostają w postaci JSON i pojawiają się w tekście przy szczegółowości `detailed` lub gdy żądanie wymienia je w `fields`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Znalezienie dosłownego tekstu w plikach katalogu.

```nurt example
search "fn main" "/home/user/app/src"
```

```text expected
/home/user/app/src/main.rs:1:1: fn main() {
```

Pokazanie nazw przechwyconych przez wyrażenie regularne, jeden rekord na dopasowanie.

```nurt example
search /fn (?<name>\w+)/ "/home/user/app/src" only_matching: true
```

```text expected
/home/user/app/src/lib.rs:1:5: fn run
/home/user/app/src/main.rs:1:1: fn main
```

Przeszukanie dziennika bez uwzględniania wielkości liter, z jednym wierszem kontekstu.

```nurt example
search "warning" "/home/user/app/logs" case_sensitive: false context: 1
```

```text expected
/home/user/app/logs/app.log:2:1: WARNING: disk almost full
```

Zliczenie dopasowań każdego wzorca z listy w każdym pliku.

```nurt example
search ["run", "main"] "/home/user/app/src" output: counts
```

Wyświetlenie plików z dopasowaniami, najpierw plików z największą liczbą dopasowań.

```nurt example
search /\brun\b|\bmain\b/ "/home/user/app" output: files sort: count
```

Odpowiedź, która pomija rekordy, kończy się wskazówką „more: next” i zawiera pole `continuation`; polecenie `next` zwraca kolejną część bez ponownego uruchamiania polecenia.

```text
matched 212  shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Brak wzorca lub brak ścieżki w postaci źródłowej. | Podaj `pattern` i `path`. |
| `E_REGEX` | Wyrażenie regularne jest niepoprawne; `found` zawiera wzorzec. | Popraw wyrażenie przy zgłoszonym znaku. |
| `E_LIMIT` | Skompilowany wzorzec jest zbyt duży, dla `sort` przechowywanych jest więcej wyników niż `max_records` albo plik powyżej limitu tekstu jest przeszukiwany z `scope`, `enclosing` lub wzorcem składni. | Uprość wzorzec, zawęź wyszukiwanie lub przeszukaj plik wzorcem tekstowym. |
| `E_UNKNOWN_VALUE` | Wartość wyliczenia lub typ pliku jest nieznany. | Użyj jednej z wymienionych wartości. |
| `E_INVALID_VALUE` | Liczba jest ujemna lub lista wzorców jest pusta. | Podaj 0 lub więcej albo co najmniej jeden wzorzec. |
| `E_TYPE` | Parametr lub rekord wejściowy ma inny typ. | Przekaż tekst, wyrażenia regularne, ścieżki albo rekordy `entry`, `content` i `line`. |
| `E_PARAM_CONFLICT` | `invert` jest połączone z `scope` lub `mode: syntax`. | Pomiń jedno z nich. |
| `E_UNSUPPORTED` | `scope` lub `mode: syntax` jest użyte bez grupy poleceń code. | Załaduj grupę code lub wyszukuj z `scope: any` i wzorcem tekstowym. |
| `E_PATH_RELATIVE` | Podano ścieżkę względną bez katalogu bazowego. | Podaj ścieżkę bezwzględną lub ustaw opcję `base`. |
| `E_NOT_FOUND` | Plik lub katalog z `path` nie istnieje. | Sprawdź ścieżkę. |
| `E_PERMISSION` | Korzenia nie można odczytać. | Sprawdź prawa dostępu do ścieżki. |
| `E_IO` | Korzenia nie można odczytać. | Sprawdź prawa dostępu do ścieżki. |

## Powiązane {#related}

`find`, `read`, `lines`, `replace`, `help skill: "text-editing"`.
