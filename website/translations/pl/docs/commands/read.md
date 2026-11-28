---
command: read
group: core
summary: Odczytuje tekst, obrazy, strony PDF, notatniki lub klucze dokumentów.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# read

## Składnia {#synopsis}

```text
read <path>... (path: <list<path>> | targets: <list<map<any>>>) [name: value]...
entry |> read [name: value]...
```

## Opis {#description}

Polecenie odczytuje pliki i tworzy jeden rekord `content` dla każdego pliku albo rekordy `item`, gdy podano `key`. W postaci źródłowej plikami są podane ścieżki, w podanej kolejności, lub mapy `targets`; każda mapa wskazuje `path` i co najwyżej jeden własny selektor. W postaci etapu plikami są pliki rekordów wejściowych `entry`; katalogi i wpisy specjalne wśród nich są pomijane.

Plik jest dekodowany w wykrytym dla niego kodowaniu albo w kodowaniu podanym w `encoding`. Zawartość, która nie jest tekstem w żadnym obsługiwanym kodowaniu, kończy się błędem `E_BINARY`, chyba że podano `view: hex`. Pole `etag` rekordu zawiera skrót SHA-256 bajtów pliku i może być przekazane do `if_match` poleceń zmieniających.

Wiersze są liczone od 1 i kończą się znakiem nowego wiersza; znak powrotu karetki przed nowym wierszem należy do zakończenia wiersza. `search` liczy wiersze tak samo. Tekst rekordu zachowuje oryginalne zakończenia wierszy.

Wywołanie przyjmuje co najwyżej jeden selektor:

| Selektor | Wynik |
|---|---|
| none | cały tekst |
| `lines` | wiersze jednego lub kilku zakresów, w podanej kolejności; nakładające się zakresy są scalane w miejscu pierwszego z nich; koniec zakresu poza ostatnim wierszem jest ograniczany do ostatniego wiersza, a zakres zaczynający się po nim kończy się błędem `E_OUT_OF_RANGE` |
| `last` | ostatnie wiersze; cały tekst, gdy plik jest krótszy |
| `around` | wiersze każdego dopasowania tekstu lub wyrażenia regularnego z `context` wierszami przed i po; zakresy, które się nakładają lub stykają, są scalane |
| `section` | sekcja Markdown: jej nagłówek ATX lub Setext oraz wiersze do następnego nagłówka tego samego lub wyższego poziomu; zagnieżdżone nagłówki są łączone przez ` > `; nagłówki w ogrodzonym kodzie są ignorowane, a znaczniki wyróżnienia, kodu i odnośników są ignorowane przy porównywaniu nagłówków |
| `symbol` | definicja symbolu kodu; nazwa kwalifikowana, taka jak `Server.start`, zawęża kontener |
| `key` | rekordy `item` wartości pod ścieżką klucza dokumentu strukturalnego, rozwinięte do `depth` poziomów |
| `cell` | jedna komórka notatnika, według indeksu od 0 lub identyfikatora |

Rekord ma `range` dla jednego zakresu wierszy i `ranges` dla żadnego lub kilku. `total_lines` to liczba wierszy pliku, a `lines` liczba wierszy w `text`.

Pliki z rozszerzeniem `.ipynb` są odczytywane jako notatniki: rekord zawiera komórki w `cells`, a ich źródła w `text`.

`view` wybiera, co zawiera rekord: `full` tekst; `header` metadane bez tekstu; `outline` symbole pliku w `symbols` i jeden wcięty wiersz na symbol w `text`; `symbols` symbole bez tekstu; `outline_first` symbole i tekst wyboru. `header`, `outline` i `symbols` nie przyjmują selektora. Bez analizy kodu lub gdy żadna gramatyka nie rozpoznaje pliku, widoki, `symbol` i `key` zwracają cały tekst i zgłaszają godny uwagi skutek `semantic_fallback`.

Odczyty warunkowe unikają wysyłania tekstu, który jest już znany. Każda z wartości `unchanged_since`, `since` i `known` jest etagiem, skrótem zawartości `sha256:…` lub co najmniej 8 początkowymi cyframi szesnastkowymi jednego z nich. Z `unchanged_since` plik, który nadal ma tę wersję, daje rekord z `unchanged: true` i bez tekstu. Z `since` rekord zawiera fragmenty różnicy względem tej wersji w `diff`, bez tekstu, gdy wersja znajduje się w pamięci podręcznej odczytu; w przeciwnym razie zawiera tekst i `since_missing: true`. Każdy odczyt tekstu zapisuje tekst w pamięci podręcznej, która przechowuje 64 MiB przez godzinę. Pliki wymienione w `known`, które nadal mają podaną wersję, są pomijane w wyniku.

`max_line` obcina dłuższe wiersze po tylu znakach i dopisuje `[N characters omitted]`; rekord ma wtedy `truncated: true`, a godny uwagi skutek `truncated_lines` podaje liczbę obciętych wierszy.

### Dokumenty PDF

Plik zaczynający się od `%PDF-` jest dokumentem PDF, także gdy składa się wyłącznie ze znaków ASCII; nazwane `encoding` odczytuje jego źródło jako tekst. Dokument PDF daje jeden rekord `content` dla każdej strony, z numerem strony w `page` i liczbą stron dokumentu w `pages_total`. `pages` wybiera strony jako jeden lub kilka zakresów, odczytywanych w podanej kolejności; koniec zakresu poza ostatnią stroną jest ograniczany do ostatniej strony, a zakres zaczynający się poza nią kończy się błędem `E_OUT_OF_RANGE`. `mode: text` wyodrębnia słowa i wiersze w kolejności czytania; `mode: layout` umieszcza każde słowo na jego poziomej pozycji na siatce znaków, co zachowuje wyrównanie kolumn i tabel. `mode: images` wymaga biblioteki PDFium i kończy się błędem `E_EXTERNAL_MISSING` w kompilacjach bez niej. `view: header` daje jeden rekord z `pages_total` i bez tekstu. Dokument wymagający hasła kończy się błędem `E_ENCRYPTED`; dokument zaszyfrowany wyłącznie w celu ograniczenia drukowania lub kopiowania jest odczytywany. Tekst pochodzi z czcionek dokumentu, więc strony zawierające tekst wyłącznie jako obrazy dają pusty tekst. Selektory wierszy nie mają zastosowania do dokumentów i kończą się błędem `E_UNSUPPORTED`.

### Obrazy

Obraz daje jeden rekord z `binary: true`, pustym `text` i `image`: typ nośnika w `mime`, szerokość i wysokość w pikselach, odczytane z nagłówka obrazu bez dekodowania pikseli, oraz bajty pliku w `data`. `view: header` odczytuje tylko nagłówek i pomija `data`. Typy rozpoznawane bez dekodera, takie jak HEIF, nie mają wymiarów.

### Zrzut szesnastkowy

`view: hex` zrzuca bajty dowolnego pliku w układzie `xxd`: przesunięcie wiersza jako osiem lub więcej cyfr szesnastkowych, dwukropek, 16 bajtów w grupach po dwa oraz te same bajty jako ASCII, z `.` dla bajtów spoza zakresu drukowalnego. Każdy wiersz jest jednym wierszem `text`; `lines`, `last` i `pages` wybierają wiersze, liczone od 1, a `total_lines` to liczba wierszy. Wiersze pliku są odczytywane według pozycji, więc zakres dużego pliku odczytuje tylko ten zakres. Tekst jednego zrzutu jest ograniczony limitem tekstu i powyżej niego kończy się błędem `E_LIMIT`.

### Pliki skompresowane

Z `decompress: auto` plik, którego nazwa kończy się na `.gz`, `.zst`, `.xz` lub `.bz2` albo na `.tgz`, `.tzst`, `.txz` lub `.tbz2` i którego zawartość zaczyna się od sygnatury tego samego formatu, jest dekompresowany w trakcie odczytu. Zdekompresowana zawartość jest odczytywana jako tekst, dokument PDF lub obraz, a `view: hex` zrzuca zdekompresowane bajty. Rekordy zachowują `path` i `etag` pliku skompresowanego i wskazują go w `decompressed_from`; zgłaszany jest godny uwagi skutek `decompressed`. Zdekompresowana zawartość większa niż limit tekstu kończy się błędem `E_LIMIT`, więc mały plik, który rozszerza się bez ograniczeń, jest zatrzymywany. `decompress: none` odczytuje bajty skompresowane. Z `since` wcześniejszy tekst pliku skompresowanego jest znajdowany tylko w pamięci podręcznej odczytu współdzielonej przez sesję.

Rekordy dokumentów PDF, obrazów i zrzutów mają `encoding: binary`, `bom: false` i `line_ending: none`. Ich `etag` zawiera skrót zawartości, gdy odczytano cały plik, co nie dotyczy zrzutów i nagłówków obrazów. `unchanged_since` i `known` mają do nich zastosowanie; `since` zwraca zawartość z `since_missing: true`.

### Wykrywanie kodowania

Z `encoding: auto` najpierw sprawdzane są sygnatury formatów binarnych: plik zaczynający się od `%PDF-` jest dokumentem PDF, plik odczytywany z `decompress: auto`, którego nazwa i sygnatura zgadzają się co do formatu kompresji, jest dekompresowany, a plik zaczynający się od sygnatury obrazu jest obrazem, chyba że jego początkowe bajty są zwykłym tekstem ASCII. Pierwsze 64 KiB każdego innego pliku rozstrzyga o jego kodowaniu, w tej kolejności: znacznik kolejności bajtów wybiera UTF-8, UTF-16 LE lub UTF-16 BE; poprawny UTF-8 bez znaków NUL to UTF-8; zawartość, w której jedna jednostka kodowa na dziesięć lub więcej ma zerowy starszy bajt, to UTF-16 bez znacznika kolejności bajtów; bajt NUL czyni zawartość binarną; w przeciwnym razie detektor statystyczny proponuje kodowanie starsze, a kodowania z klucza konfiguracji `text.legacy_encodings` (domyślnie `windows-1250`, `ibm852`, `iso-8859-2` i `windows-1252`) zastępują propozycję, gdy któreś z nich odczytuje zawartość jako tekst polski. Zawartość z więcej niż jednym znakiem sterującym na dwadzieścia znaków jest binarna.

- Wykrywane kodowania starsze to `windows-1250`, `ibm852`, `iso-8859-2` i `windows-1252`. Zawartość w innej stronie kodowej, takiej jak `windows-1251`, jest odczytywana w najbardziej prawdopodobnej z nich.
- Próbki krótsze niż 200 bajtów zawierają niewiele liter spoza ASCII, więc wybór między kodowaniami starszymi jest dla nich mniej pewny; dla takich plików podaj `encoding`.
- UTF-16 bez znacznika kolejności bajtów jest rozpoznawany tylko dla tekstu, w którym co najmniej jeden znak na dziesięć leży w zakresie U+0000 do U+00FF, takiego jak tekst w piśmie łacińskim; inny tekst UTF-16 bez znacznika kolejności bajtów jest binarny.
- Dokument PDF składający się wyłącznie ze znaków ASCII jest odczytywany jako dokument PDF; nazwane `encoding` odczytuje jego źródło jako tekst.
- Sygnatury obrazów składające się z liter ASCII, takie jak `BM` i `GIF`, nie mają zastosowania do zwykłego tekstu ASCII, więc plik tekstowy zaczynający się od nich jest odczytywany jako tekst.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `list<path>` | jeden z `target` | Pliki do odczytu. |
| `lines` | `range \| list<range>` | none | Zakresy wierszy do odczytu. |
| `last` | `int` | none | Liczba ostatnich wierszy do odczytu. |
| `around` | `text \| regex` | none | Odczytuje wiersze wokół dopasowań tego wzorca. |
| `context` | `int` | `3` | Liczba wierszy kontekstu wokół dopasowań around. |
| `symbol` | `text` | none | Odczytuje definicję tego symbolu kodu. |
| `section` | `text` | none | Odczytuje sekcję Markdown o tym nagłówku. |
| `cell` | `int \| text` | none | Komórka notatnika według indeksu lub identyfikatora. |
| `targets` | `list<map<any>>` | jeden z `target` | Selektory dla każdego celu: path, lines, around, symbol, key, section, cell. |
| `key` | `key` | none | Ścieżka klucza w dokumencie strukturalnym; wynikiem są rekordy item. |
| `depth` | `int` | none | Głębokość zagnieżdżonych wartości odczytywanych pod key. |
| `view` | `enum(full\|outline\|outline_first\|symbols\|header\|hex)` | `full` | Pełna zawartość, zarys, symbole, nagłówek lub zrzut szesnastkowy. |
| `pages` | `range \| list<range>` | none | Strony PDF do odczytu. |
| `mode` | `enum(text\|layout\|images)` | `text` | Tryb wyodrębniania stron PDF. |
| `encoding` | `text` | `"auto"` | Kodowanie tekstu; auto wykrywa je automatycznie. |
| `decompress` | `enum(auto\|none)` | `auto` | Odczytuje pliki skompresowane po ich dekompresji w locie. |
| `unchanged_since` | `text` | none | Etag; niezmieniony plik jest zwracany bez tekstu. |
| `since` | `text` | none | Etag wcześniejszego odczytu; wynik zawiera różnice od tego czasu. |
| `known` | `map<text>` | none | Mapa ze ścieżki na etag; niezmienione pliki są pomijane. |
| `line_numbers` | `bool` | `false` | Poprzedza wiersze ich numerami. |
| `max_line` | `int` | brak limitu | Maksymalna długość zwracanego wiersza w znakach; dłuższe wiersze są obcinane i oznaczane. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| `source` | `none` | `content`, `item` |
| `stage` | `entry` | `content`, `item` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Odczyt wierszy wokół początku `main` z trzema wierszami kontekstu i numerami wierszy.

```nurt example
read "/srv/app/main.go" around: /func main/ context: 3 line_numbers: true
```

```text expected
41  func helper() {}
42  
43  // main starts the server.
44  func main() {
45  	helper()
46  }
47  
```

Ponowny odczyt pliku tylko wtedy, gdy zmienił się od wersji, której skrót zawartości zaczyna się od `1a2b3c4d`.

```nurt example
read "/srv/app/main.go" unchanged_since: "1a2b3c4d"
```

```text expected
unchanged: true
```

Odczyt kilku zakresów wierszy; nakładające się zakresy `5..8` i `7..10` są scalane w miejscu pierwszego.

```nurt example
read "/srv/app/CHANGELOG.md" lines: [5..8, 1..2, 7..10]
```

```text expected
ranges: [5..10, 1..2]
```

Odczyt podsekcji `Linux` sekcji `Install` pliku Markdown.

```nurt example
read "/srv/app/README.md" section: "Install > Linux"
```

```text expected
### Linux

apt install app

```

Odczyt ostatnich dwóch wierszy dziennika.

```nurt example
read "/var/log/app.log" last: 2
```

```text expected
request 2
stop
```

Odczyt stron 2 i 3 dokumentu PDF.

```nurt example
read "/srv/docs/report.pdf" pages: 2..3
```

```text expected
page: 2, pages_total: 3
Revenue grew in the third quarter.
page: 3, pages_total: 3
Costs fell by four percent.
```

Zrzut pierwszych dwóch wierszy bajtów pliku wykonywalnego.

```nurt example
read "/srv/app/bin/app" view: hex lines: 1..2
```

```text expected
00000000: 7f45 4c46 0201 0100 0000 0000 0000 0000  .ELF............
00000010: 0300 3e00 0100 0000 4010 0000 0000 0000  ..>.....@.......
```

Odczyt ostatniego wiersza skompresowanego dziennika.

```nurt example
read "/var/log/app.log.gz" last: 1
```

```text expected
stop
decompressed_from: /var/log/app.log.gz
```

Odczyt typu i wymiarów obrazu bez jego danych.

```nurt example
read "/srv/site/logo.png" view: header
```

```text expected
image: {mime: image/png, width: 64, height: 32}
```

Odpowiedź, która pomija wiersze, kończy się wskazówką „more: next” i zawiera pole `continuation`; polecenie `next` zwraca kolejną część bez ponownego uruchamiania polecenia.

```text
shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | Plik nie istnieje. | Sprawdź ścieżkę, na przykład poleceniem `find`. |
| `E_PERMISSION` | Pliku nie można odczytać. | Odczytaj plik, który konto może odczytać. |
| `E_INVALID_VALUE` | Podano katalog lub plik specjalny, numer wiersza jest mniejszy niż 1, `last` lub `max_line` jest mniejsze niż 1, wartość nie jest etagiem albo podano `cell` dla pliku, który nie jest notatnikiem. | Podaj plik lub poprawną wartość. |
| `E_OUT_OF_RANGE` | Zakres `lines` zaczyna się po ostatnim wierszu pliku, co dotyczy każdego zakresu pustego pliku, albo indeks `cell` nie jest mniejszy niż liczba komórek. `found` zawiera zakres, a `actual` liczbę wierszy lub komórek. | Użyj wierszy od 1 do `total_lines` lub indeksu komórki mniejszego niż `actual`. |
| `E_PARAM_CONFLICT` | Więcej niż jeden selektor, selektory zarówno dla wywołania, jak i w `targets`, albo selektor z `view: header`, `outline` lub `symbols`. | Użyj jednego selektora lub `view: outline_first`. |
| `E_MISSING_PARAM` | Brak zarówno `path`, jak i `targets`, albo cel bez `path`. | Podaj pliki do odczytu. |
| `E_UNKNOWN_FIELD` | Mapa `targets` ma nieznany klucz. | Użyj kluczy wymienionych w `expected`. |
| `E_BINARY` | Zawartość nie jest tekstem. | Odczytaj ją z `view: hex`. |
| `E_ENCODING` | Zawartość nie jest poprawna w kodowaniu podanym w `encoding`. | Użyj `encoding: auto` lub kodowania pliku. |
| `E_LIMIT` | Tekst jest większy niż limit tekstu. | Przeszukaj plik poleceniem `search`. |
| `E_REGEX` | Wzorzec `around` nie jest poprawnym wyrażeniem regularnym. | Popraw wzorzec. |
| `E_KEY_NOT_FOUND` | Sekcja lub klucz nie istnieje albo żadna komórka nie ma identyfikatora podanego w `cell`. | Użyj nagłówka wymienionego w `expected` albo istniejącego klucza lub identyfikatora komórki. |
| `E_SYMBOL_NOT_FOUND` | Plik nie definiuje takiego symbolu. | Wyświetl symbole z `view: symbols`. |
| `E_SYMBOL_AMBIGUOUS` | Nazwa oznacza kilka symboli. | Zakwalifikuj nazwę jej kontenerem. |
| `E_UNSUPPORTED_LANGUAGE` | Żadna gramatyka nie rozpoznaje pliku podanego z `symbol`. | Odczytaj zamiast tego wiersze lub sekcję. |
| `E_UNSUPPORTED` | Plik podany z `key` nie jest dokumentem strukturalnym znanego formatu. | Odczytaj zamiast tego wiersze. |
| `E_FORMAT` | Notatnik nie jest poprawnym JSON formatu notatnika. | Odczytaj jego tekst z `lines`, co nie analizuje notatnika. |

`last` większe niż liczba wierszy daje cały tekst, a `around` bez dopasowania daje pusty tekst; żadne z nich nie jest błędem.

### Błędy dokumentów PDF, obrazów, zrzutów i plików skompresowanych

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_OUT_OF_RANGE` | Zakres `pages` zaczyna się po ostatniej stronie dokumentu. | Użyj stron od 1 do `pages_total`. |
| `E_INVALID_VALUE` | Numer strony lub wiersza w `pages` jest mniejszy niż 1. | Licz strony i wiersze od 1. |
| `E_ENCRYPTED` | Dokument PDF wymaga hasła. | Odczytaj kopię bez hasła. |
| `E_FORMAT` | Dokument PDF, nagłówek obrazu lub skompresowane dane są uszkodzone. | Sprawdź, czy plik jest kompletny, lub odczytaj go z `decompress: none` albo `view: hex`. |
| `E_EXTERNAL_MISSING` | Podano `mode: images` w kompilacji bez biblioteki PDFium. | Użyj `mode: text` lub `mode: layout`. |
| `E_UNSUPPORTED` | Podano selektor wierszy lub widok tekstu dla dokumentu PDF lub obrazu albo selektor inny niż `lines` i `last` z `view: hex`. | Wybierz strony za pomocą `pages` lub wiersze za pomocą `lines`, `last` lub `pages`. |
| `E_PARAM_CONFLICT` | `view: hex` z więcej niż jednym z `lines`, `last` i `pages`. | Wybierz wiersze jednokrotnie. |
| `E_LIMIT` | Dokument, obraz, zdekompresowana zawartość lub zrzut jest większy niż limit tekstu. | Zrzuć wybrane wiersze z `view: hex` i `lines` albo odczytaj z `decompress: none`. |

## Powiązane {#related}

`lines` tworzy jeden rekord `line` dla każdego wiersza, `search` znajduje dopasowania w wielu plikach, `info` zgłasza metadane, `notebook.cells` wyświetla komórki notatnika, a `diff` porównuje dwa pliki.
