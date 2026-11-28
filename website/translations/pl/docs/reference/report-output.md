<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/reference/report-output.md -->
<!-- Responsibility:   Documents Report output. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Wyjście raportu {#report-output}

Wynikiem programu jest raport `nurt-report/1`. Skrzynka `nurt-report` redukuje
raport do żądanego poziomu szczegółowości i renderuje go w jednym z czterech
formatów. Koperta odpowiedzi `nurt-response/1` zawiera zredukowany raport i
wyrenderowany tekst.

## Opcje wywołania {#call-options}

| Opcja | Typ | Wartość domyślna | Skutek |
|---|---|---|---|
| `format` | `text`, `json`, `markdown`, `compact` | `text` | Format wyrenderowanego wyniku. |
| `verbosity` | `quiet`, `normal`, `detailed` | `normal` | Poziom szczegółowości raportu. |
| `limit` | liczba całkowita, od 1 do 5000 | `50` | Liczba zmian lub rekordów na stronę. |
| `diff` | `none`, `stat`, `unified` | `unified` | Postać różnic. |
| `diff_context` | liczba całkowita | `3` | Wiersze kontekstu różnicy; ustalane przy tworzeniu fragmentów, renderer ich nie zmienia. |
| `max_chars` | liczba całkowita | `0` | Największa liczba znaków jednego pola tekstowego; `0` oznacza brak limitu. |
| `fields` | lista nazw | wszystkie pola | Pola zachowane w wierszach programu odczytu. |
| `save_to` | ścieżka bezwzględna | brak | Plik, który otrzymuje pełny wynik; raport podaje `result_file`. |

Klucze konfiguracji `report.diff_max_hunks` (domyślnie `20`) i
`report.diff_max_lines` (domyślnie `200`) ograniczają różnice przy szczegółowości
`normal`.

Szerokość terminala i kolory dotyczą wyłącznie formatu `text` wiersza poleceń.
Tekst odpowiedzi nie zawiera sekwencji ANSI i nie jest skracany do szerokości
terminala.

## Szczegółowość {#verbosity}

| Poziom | Zachowane | Usunięte |
|---|---|---|
| `quiet` | status, `op`, cele, podsumowanie, `notable`, `irreversible`, `error`, `result_file`, rekordy programu odczytu | zmiany, różnice, `bulk`, `timing` |
| `normal` | wszystko z `quiet`, zmiany do `limit`, różnice do `report.diff_max_hunks` fragmentów i `report.diff_max_lines` wierszy, `bulk` | `timing` |
| `detailed` | wszystko z `normal`, wszystkie różnice, `timing` | brak |

Liczba pominiętych zmian jest zapisywana jako wiersz `… N more changes`;
nagłówek różnic podaje łączną liczbę fragmentów. Tryb różnic inny niż `unified`
usuwa różnice; liczby dodanych i usuniętych wierszy pozostają w zmianach.

## Limity {#limits}

- Zmiany ponad `limit` są liczone, a nie wymieniane. `targets.listed` jest
  ustawiane na liczbę wymienionych zmian.
- Wiersze programu odczytu ponad `limit` są liczone, a nie wymieniane. Kursorem
  następnej strony jest `next_cursor`, ustawiany przez silnik. Pole
  `continuation` (`id`, `unit`, `shown`, `remaining`, `cut_lines`) nazywa
  zachowaną resztę, a tekst odpowiedzi kończy się `more: next`; polecenie `next`
  odczytuje z niego następną część.
- Różnice są ograniczane tak, jak podano w sekcji Szczegółowość.

## Skracanie pól {#field-shortening}

Pole tekstowe dłuższe niż `max_chars` znaków jest obcinane na granicy znaku.
Rekord zawierający to pole otrzymuje `truncated: true`. Reguła dotyczy rekordów
`exec`, `job`, `test`, `analysis`, `git` oraz nagłówka i wierszy programu
odczytu, w tym zagnieżdżonych list, map i rekordów.

## Wybór pól {#field-selection}

`fields` zachowuje w każdym wierszu programu odczytu wyłącznie wymienione pola,
w kolejności z opcji. Nieznane nazwy są ignorowane. Rekord nagłówka i pozostałe
rekordy raportu nie są dotknięte.

## Formaty {#formats}

### text {#text}

Jeden blok na część raportu, w stałej kolejności: poprawki, `ran`, nagłówek,
cele, blok programu lub zadania, rekordy, zmiany, różnice, podsumowanie masowe,
rekordy testu, analizy i git, `notable`, `irreversible`, błąd, następny kursor,
plik wyniku, pomiar czasu. Liczby są grupowane spacją co trzy cyfry. Rozmiary
używają jednostek binarnych (`b`, `kib`, `mib`, `gib`, `tib`).

```
op 18  replace  ok
targets 3 files · replacements 7 · lines +7 −7 · 41.2kib → 41.3kib
  /srv/app/src/a.go    3 repl  1.1kib → 1.1kib    1a2b3c4d → 9f8e7d6c
diff (3 hunks)
  /srv/app/src/a.go:12
    - func oldName(x int) {
    + func newName(x int) {
notable
  ! many_per_file  /srv/app/src/c/d.go  2  many replacements in one file
next cursor c_5d2e
```

Wiersz `notable` zawiera kod, cel, liczbę i szczegół wpisu. Wpis bez szczegółu
pokazuje standardowy komunikat swojego kodu.

Program odczytu zapisuje nagłówek `<kind>  matched N · shown N · next cursor
C` oraz tabelę wierszy. Tabela rekordów `operation` (`history`), `match`
(`search`) i `response` (`http.request`) zawiera wyłącznie kolumny podstawowe;
szczegółowość `detailed` lub opcja `fields` pokazuje pozostałe, a postać JSON
zawsze zawiera wszystkie pola. Nieudany program bez numeru operacji zaczyna się
od wiersza `error <code>`. Raport ze statusem `ok` i błędem (niepowodzenie po
skutku) zapisuje najpierw skutek, a po nim blok błędu z wierszem `effect applied`.

Wiersz `result file <path>` pojawia się, gdy wynik został zapisany do pliku.

Nagłówki i etykiety formatów `text`, `markdown` i `compact` są słowami
kluczowymi i pozostają angielskie niezależnie od parametru wywołania `language`;
`language` zmienia komunikaty, które producent raportu umieszcza w swoich danych.

Znak sterujący w wierszu różnicy, taki jak powrót karetki pliku z zakończeniami
wierszy CRLF, jest zapisywany jako sekwencja ucieczki (`\r`) w formatach `text` i
`compact`; tabulator pozostaje bez zmian.

### json {#json}

Zredukowany raport jako jeden wiersz JSON, który przechodzi walidację względem
`nurt-report/1`. Kolejność pól jest stała. Odpowiedź nie ma składnika `rendered`
dla tego formatu, ponieważ wynikiem jest sam raport.

### markdown {#markdown}

Nagłówki, tabele z kreskami pionowymi i bloki ogrodzone dla różnic i drzew.
Części są te same co w formacie `text`. Plik wyniku jest zapisywany jako
``result file `<path>` ``.

### compact {#compact}

Jeden rekord na wiersz, bez nagłówków. Pierwszy wiersz to
`report kind=<kind> status=<status>` z `op` i `ran`, gdy są obecne. Kolejne
wiersze zaczynają się od nazwy części: `correction`, `targets`, `summary`,
wiersz zmiany, `diff <path>:<line> <text>`, `bulk`, `notable`, `irreversible`,
`error`, `next cursor=<cursor>`, `result file=<path>`, `timing`. Wartość jest
zapisywana jako `key=value`; wartość pusta lub zawierająca spację, cudzysłów
albo `=` jest zapisywana jako ciąg JSON.

## Widok drzewa {#tree-view}

Wiersze rodzaju `entry`, które wszystkie zawierają całkowitą `depth`, są
rysowane jako drzewo w formatach `text` i `markdown`. Wiersz o głębokości 0 jest
korzeniem. Wiersz o głębokości `d` jest dzieckiem najbliższego poprzedzającego
wiersza o głębokości `d - 1`. Łączniki to `|-- `, `` `-- `` i `|   `. Katalog
jest zapisywany z końcowym `/`, plik z rozmiarem. Katalog nad wierszem, którego
nie ma na liście, jak wtedy, gdy `find` zwraca tylko dopasowania, jest
wyprowadzany ze ścieżki `path` wiersza i rysowany na swojej głębokości; wartość
z podziałem wiersza jest zapisywana w jednym wierszu, a podział jako `\n`.

```
src/
|-- a.go  1.1kib
`-- b.go  8.0kib
```

Formaty `json` i `compact` zachowują płaską listę wierszy z ich `depth`.

## Podziały wierszy i znaki sterujące {#line-breaks-and-control-characters}

Raport jest wyświetlany wiersz po wierszu, więc wartość pochodząca spoza raportu
(nazwa pliku, ścieżka, komórka rekordu, komunikat) jest zapisywana w jednym
wierszu w formatach `text`, `markdown` i `compact`. Wysunięcie wiersza jest
zapisywane jako `\n`, powrót karetki jako `\r`, tabulator jako `\t`, a każdy
inny znak sterujący jako sekwencja ucieczki Rust, taka jak `\u{1}`. Odwrotny
ukośnik nie jest podwajany, więc nazwa z dwoma znakami `\n` i nazwa z wysunięciem
wiersza wyglądają tak samo; format `json` je rozróżnia.

| Miejsce | Reguła |
|---|---|
| komórka tabeli formatu `text` | zapisana z ucieczką jak wyżej |
| widok drzewa | etykieta wiersza jest zapisana z ucieczką; katalog wyprowadzony ze `path` jest zapisany z ucieczką tak samo |
| komórka tabeli formatu `markdown` | zapisana z ucieczką jak wyżej, następnie `\|` dla kreski pionowej |
| kod w wierszu i element listy formatu `markdown` | zapisane z ucieczką jak wyżej; ogranicznik kodu jest dłuższy niż każdy ciąg odwrotnych apostrofów w tekście |
| wartość formatu `compact` | wartość ze spacją, znakiem sterującym, cudzysłowem lub `=` jest zapisywana jako ciąg JSON |
| ścieżka wiersza `diff` formatu `compact` | zapisana z ucieczką jak wyżej |

Tekst pliku pokazywany przez `read` jest jedną wartością jak każda inna: w tabeli
jest zapisywany w jednym wierszu, z `\n` dla każdego podziału wiersza, a format
`json` zachowuje wiersze.

## Ścieżki niebędące prawidłowym Unicode {#paths-that-are-not-valid-unicode}

Ścieżka lub nazwa z bajtami, które nie są prawidłowym Unicode (w systemie Linux
na przykład bajt `0xFF`), jest pokazywana w każdym formacie z nieprawidłowymi
bajtami zastąpionymi przez U+FFFD. Format `json` dodaje postać surową obok
postaci wyświetlanej: pole `raw_<field>` typu `bytes` (standardowy base64),
które zawiera natywne bajty ścieżki, na przykład `raw_path` obok `path` w
`change`, `hunk`, `entry`, `match` i we wpisie `notable` lub `irreversible`,
`raw_directory` w katalogach podsumowania masowego oraz `raw_sample_head` i
`raw_sample_tail` w jego próbce. Pole surowe jest nieobecne, gdy ścieżka jest
prawidłowym Unicode. Postać surowa jest też tym, co polecenia odczytują z
otrzymanych rekordów, więc `find ... |> remove` działa na wpisie, który wymienił
`find`. Dziennik przechowuje postać surową każdego celu, więc `undo` przywraca
wpis pod jego nazwą. W formatach `text`, `markdown` i `compact` rekord pokazuje
pole surowe jako liczbę jego bajtów, `<N bytes>`.

## Koperta odpowiedzi {#response-envelope}

| Składnik | Typ | Wartość |
|---|---|---|
| `nurt` | ciąg znaków | `nurt-response/1` |
| `report` | obiekt | raport po redukcji |
| `rendered` | obiekt | `format` i `text` wyrenderowanego raportu; nieobecny dla formatu `json` |

`report.next_cursor` jest kursorem następnej strony.

## Błędy {#errors}

Błąd raportu jest zapisywany z polami `code`, `at`, `fix`, `help`, `expected`,
`actual`, `sample`, `candidates`, `next_steps` i `cause`. Każdy kod wyliczenia
błędów daje raport, który przechodzi walidację względem `nurt-report/1` i
renderuje się we wszystkich czterech formatach.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
