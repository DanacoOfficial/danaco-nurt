<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           spec/records.md -->
<!-- Responsibility:   Documents Values and records. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Wartości i rekordy {#values-and-records}

Programy wymieniają typowane wartości i rekordy. Ta specyfikacja definiuje typy wartości, ich postać JSON `nurt-value/1` oraz każdy rodzaj rekordu wraz z jego polami. Schemat `nurt-value-1.schema.json` jest generowany z typów crate'a `nurt-contracts`; test crate'a porównuje oba i waliduje pliki wzorcowe (golden) z `spec/golden/value` względem schematu.

## Typy wartości {#value-types}

| Typ | Znaczenie | Ładunek w JSON z tagiem | Zwykły JSON |
|---|---|---|---|
| `null` | brak wartości | brak | `null` |
| `bool` | wartość logiczna | `true` lub `false` | tak samo |
| `int` | 64-bitowa liczba całkowita ze znakiem; większe liczby całkowite odczytane z dokumentów są typu `big_int` z cyframi dziesiętnymi | liczba lub tekst dla `big_int` | liczba |
| `number` | skończona 64-bitowa liczba zmiennoprzecinkowa; dokładny tekst źródłowy odczytany z dokumentu jest typu `decimal` | liczba lub tekst dla `decimal` | liczba |
| `text` | tekst Unicode | tekst | tekst |
| `size` | rozmiar w bajtach | liczba całkowita bez znaku | liczba całkowita |
| `duration` | czas trwania ze znakiem w milisekundach | liczba całkowita | liczba całkowita |
| `instant` | punkt w czasie w UTC z precyzją nanosekundową, od 1677 do 2262 | tekst RFC 3339 z `Z` i 0, 3, 6 lub 9 cyframi ułamka | tak samo |
| `date` | data lub czas odczytane z dokumentu, zachowane w postaci źródłowej | `{"kind", "text"}` | tekst |
| `path` | ścieżka w postaci kanonicznej: `/` jako separator, bezwzględna, wielka litera dysku, prefiksy dostawców `registry:`, `env:`, `archive:` | tekst | tekst |
| `glob` | wzorzec nazw plików | tekst | tekst |
| `regex` | wyrażenie regularne | `{"pattern", "flags"}`, flagi w kolejności `imsx`, pomijane, gdy puste | `/pattern/flags` |
| `key` | ścieżka klucza w dokumencie strukturalnym | tekst | tekst |
| `bytes` | dane binarne | standardowy base64 z dopełnieniem | tak samo |
| `range` | domknięty zakres liczb całkowitych; obie granice opcjonalne, początek nie większy niż koniec | `{"start", "end"}` | `1..5`, `40..`, `..9`, `5` |
| `enum` | wartość wyliczenia | tekst | tekst |
| `list` | uporządkowana lista | tablica wartości z tagiem | tablica |
| `map` | mapa z uporządkowanymi kluczami | obiekt wartości z tagiem | obiekt |
| `record` | rekord danego rodzaju | `{"kind", "fields"}` | obiekt pól |

Postać JSON z tagiem jest obiektem z typem w `t` i ładunkiem w `v`, na przykład `{"t": "size", "v": 1048576}`; `null` to `{"t": "null"}`. Postać z tagiem jest bezstratna i zachowuje kolejność kluczy map oraz pól rekordów. Postać zwykła jest używana dla pól rekordów typowanych i dla wierszy w raportach; pomija tagi typów, które wynikają z rodzaju rekordu.

## Rekordy {#records}

Rekord ma rodzaj i pola w ustalonej kolejności. Pole oznaczone jako wymagane jest zawsze obecne; każde inne pole jest obecne tylko wtedy, gdy ma sens lub zostało obliczone. W zwykłej postaci JSON rekordu pola nieobecne są pomijane, rozmiary są podawane w bajtach, czasy trwania w milisekundach, znaczniki czasu (`instant`) jako tekst RFC 3339 w UTC, a pola typu `any` używają postaci z tagiem. Rekord typowany dowolnego rodzaju jest zapisywany jako `{"kind": "entry", "record": {...}}`; postać dynamiczna używana przez etapy działające na polach według nazw to `{"kind": "entry", "fields": {...}}` z wartościami z tagiem.

### Ścieżki niebędące poprawnym Unicode {#paths-that-are-not-valid-unicode}

Ścieżka jest tekstem w postaci kanonicznej, więc ścieżka zawierająca bajty niebędące poprawnym Unicode (nazwa uniksowa z bajtem `0xFF`) jest zapisywana z każdym takim bajtem zastąpionym przez U+FFFD, czyli w postaci do wyświetlania. Tekst ten nie prowadzi z powrotem do wpisu, dlatego każde pole ścieżki rekordu, które może przechowywać taką ścieżkę, ma pole towarzyszące `raw_<field>` typu `bytes` (`raw_path` dla `path`, `raw_parent` dla `parent`, `raw_from` dla `from`). Pole towarzyszące zawiera natywne bajty całej ścieżki w postaci przyjmowanej przez system: w systemie Unix bajty ścieżki, bezwzględne, z separatorami `/`. Jest obecne tylko wtedy, gdy ścieżka nie jest poprawnym Unicode; dla każdej innej ścieżki rekord ma wyłącznie pole do wyświetlania. Pola towarzyszące mają tylko systemy, w których ścieżki są łańcuchami bajtów (Unix); ścieżka systemu Windows jest zawsze pokazywana w postaci do wyświetlania. `raw_name` jest starsze i obejmuje także nazwę, która jest poprawnym Unicode, lecz nie jest w NFC.

Polecenie przyjmujące rekordy poprzedniego etapu odczytuje ścieżkę rekordu z `raw_path`, gdy rekord ją ma, a jego `path` jest nadal postacią do wyświetlania tej ścieżki, a w przeciwnym razie z `path`, tak aby `find ... |> remove`, `move`, `copy`, `replace` i pozostałe polecenia działały na wpis, który został wylistowany. Ścieżka zapisana w programie jest tekstem i nie może wskazać takiego wpisu; do wpisu dociera się przez rekordy `list`, `find`, `info` lub `search`. Dziennik przechowuje natywne bajty celów operacji, więc `undo` i `redo` przywracają wpis pod jego pierwotną nazwą. Raporty pokazują postać do wyświetlania w formatach `text`, `markdown` i `compact`, a postać surową podają w formacie `json`.

Rekordy poleceń systemu plików (`entry`, `content`, `line`, `match`, `hunk`, `change` oraz elementy `operation`) i rekordy `git_status` (`raw_path`, `raw_renamed_from`) oraz `git_conflict` (`raw_path`), które przechowują ścieżki względem korzenia repozytorium, podlegają tej regule; rekordy innych rodzin, na przykład `symbol`, `item` lub `cell`, pokazują wyłącznie postać do wyświetlania. Pola raportu nazywające ścieżkę podlegają tej samej regule: `raw_path`, `raw_from`, `raw_to` i `raw_link_target` rekordu [`change`](#change), `raw_path` rekordu [`hunk`](#hunk), wpisów `notable` i `irreversible`, `raw_directory` katalogów zbiorczego podsumowania oraz `raw_sample_head` i `raw_sample_tail` jego próbki (jeden element dla każdego celu próbki, `null` dla celu, który jest poprawnym Unicode).

| Rodzaj | Opis |
|---|---|
| [`entry`](#entry) | Wpis systemu plików: plik, katalog, dowiązanie lub inny obiekt. |
| [`content`](#content) | Treść tekstowa pliku, zdekodowana do UTF-8. |
| [`line`](#line) | Jeden wiersz pliku. |
| [`match`](#match) | Dopasowanie wyszukiwania. |
| [`item`](#item) | Wartość pod ścieżką klucza w dokumencie strukturalnym. |
| [`row`](#row) | Wiersz tabeli, pliku CSV lub zapytania. |
| [`node`](#node) | Węzeł drzewa składni. |
| [`symbol`](#symbol) | Symbol kodu źródłowego. |
| [`reference`](#reference) | Odwołanie do symbolu. |
| [`diagnostic`](#diagnostic) | Diagnostyka kompilatora, lintera lub serwera języka. |
| [`cell`](#cell) | Komórka notatnika. |
| [`hunk`](#hunk) | Fragment różnicy w formacie unified. |
| [`change`](#change) | Zmiana jednego pliku w ramach skutku. |
| [`exec`](#exec) | Wynik zakończonego uruchomienia programu. |
| [`job`](#job) | Zadanie w tle. |
| [`process`](#process) | Proces systemu operacyjnego. |
| [`port`](#port) | Port sieciowy. |
| [`disk`](#disk) | Zamontowany dysk. |
| [`host`](#host) | Opis urządzenia. |
| [`environment_variable`](#environment_variable) | Zmienna środowiskowa; wartość nigdy nie jest maskowana. |
| [`service`](#service) | Usługa systemu operacyjnego. |
| [`package`](#package) | Pakiet zainstalowany lub dostępny. |
| [`account`](#account) | Konto użytkownika lub grupy. |
| [`log_event`](#log_event) | Zdarzenie dziennika systemowego. |
| [`commit`](#commit) | Commit repozytorium. |
| [`git_status`](#git_status) | Stan ścieżki w repozytorium. |
| [`branch`](#branch) | Gałąź repozytorium. |
| [`schedule`](#schedule) | Zaplanowane zadanie. |
| [`operation`](#operation) | Wpis dziennika operacji. |
| [`response`](#response) | Odpowiedź otrzymana na żądanie HTTP. |
| [`volume`](#volume) | Wolumin widziany przez dziennik. |
| [`program_card`](#program_card) | Opis programu. |
| [`archive_entry`](#archive_entry) | Element archiwum. |
| [`test_run`](#test_run) | Przebieg testów. |
| [`test_result`](#test_result) | Wynik jednego testu. |
| [`finding`](#finding) | Ustalenie analizy lub przeglądu. |
| [`effect`](#effect) | Skutek etapu zmieniającego lub uruchamiającego, jako rekord dla etapów następujących po skutku. |
| [`git_effect`](#git_effect) | Skutek operacji na repozytorium. |
| [`registry_value`](#registry_value) | Wartość w rejestrze systemu Windows. |
| [`journal_state`](#journal_state) | Stan i limity dziennika operacji. |
| [`value`](#value) | Wynik wyrażenia. |
| [`group`](#group) | Grupa rekordów o tym samym kluczu. |
| [`summary`](#summary) | Statystyki pola. |
| [`field_info`](#field_info) | Pole rekordów w potoku. |
| [`cim_object`](#cim_object) | Obiekt zarządzania systemu Windows. |
| [`blame_line`](#blame_line) | Wiersz opatrzony informacją o commicie, który zmienił go jako ostatni. |
| [`hover`](#hover) | Opis symbolu w danej pozycji. |
| [`code_action`](#code_action) | Akcja kodu oferowana przez serwer języka. |
| [`language`](#language) | Obsługiwany język i jego serwer. |
| [`rename_edit`](#rename_edit) | Planowana edycja zmiany nazwy. |
| [`profile_entry`](#profile_entry) | Wpis profilu repozytorium. |
| [`ping_result`](#ping_result) | Wynik testu osiągalności; czasy w milisekundach. |
| [`dns_record`](#dns_record) | Rekord zasobu DNS. |
| [`network_interface`](#network_interface) | Interfejs sieciowy. |
| [`program_location`](#program_location) | Lokalizacja programu. |
| [`program_category`](#program_category) | Kategoria programów. |
| [`schedule_run`](#schedule_run) | Jedno uruchomienie zaplanowanego zadania. |
| [`analysis`](#analysis) | Werdykt analizy. |
| [`analyze_tool`](#analyze_tool) | Narzędzie analizy znalezione w projekcie. |
| [`git_tag`](#git_tag) | Tag repozytorium. |
| [`git_worktree`](#git_worktree) | Drzewo robocze repozytorium. |
| [`git_stash`](#git_stash) | Wpis schowka (stash). |
| [`git_checkpoint`](#git_checkpoint) | Punkt kontrolny drzewa roboczego. |
| [`git_ref_update`](#git_ref_update) | Aktualizacja referencji przez transfer. |
| [`git_conflict`](#git_conflict) | Konflikt pozostawiony przez scalenie. |
| [`git_trash_entry`](#git_trash_entry) | Zachowana kopia usuniętej gałęzi lub odrzuconego wpisu schowka. |
| `map` | Rekord pól obliczanych lub zdefiniowanych w źródle, bez ustalonego rodzaju, na przykład wynik `select` z polami obliczanymi. |

### entry {#entry}

Wpis systemu plików: plik, katalog, dowiązanie lub inny obiekt.

Dla wpisu, który nie istnieje (`exists: false` w wyniku `info`), `size` wynosi zero, a `modified` jest epoką uniksową.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Ścieżka bezwzględna w postaci kanonicznej. |
| `raw_path` | bytes | nie | Natywne bajty `path`; obecne tylko wtedy, gdy ścieżka nie jest poprawnym Unicode. |
| `name` | text | tak | Ostatni składnik ścieżki, w NFC. |
| `stem` | text | nie | Nazwa bez ostatniego rozszerzenia. |
| `extension` | text | nie | Ostatnie rozszerzenie, bez początkowej kropki. |
| `parent` | path | nie | Katalog nadrzędny. |
| `raw_parent` | bytes | nie | Natywne bajty `parent`; obecne tylko wtedy, gdy nie jest poprawnym Unicode. |
| `type` | enum(file\|directory\|link\|fifo\|device\|other) | tak | Typ wpisu. |
| `exists` | bool | nie | Czy wpis istnieje; ustawiane w wyniku `info`. |
| `size` | size | tak | Rozmiar w bajtach; dla katalogów zgodnie z raportem systemu. |
| `modified` | instant | tak | Czas ostatniej modyfikacji. |
| `created` | instant | nie | Czas utworzenia. |
| `accessed` | instant | nie | Czas ostatniego dostępu. |
| `mode` | text | nie | Bity uprawnień w zapisie ósemkowym, na przykład `0644`. |
| `owner` | text | nie | Konto właściciela: nazwa i identyfikator. |
| `group` | text | nie | Grupa właściciela. |
| `acl` | list<text> | nie | Wpisy kontroli dostępu. |
| `readonly` | bool | nie | Tylko do odczytu. |
| `hidden` | bool | nie | Ukryty. |
| `link_target` | path | nie | Cel dowiązania. |
| `raw_link_target` | bytes | nie | Natywne bajty `link_target`; obecne tylko wtedy, gdy nie jest poprawnym Unicode. |
| `link_type` | enum(symbolic\|junction\|hard) | nie | Rodzaj dowiązania. |
| `major` | int | nie | Główny numer urządzenia węzła urządzenia. |
| `minor` | int | nie | Poboczny numer urządzenia węzła urządzenia. |
| `links` | int | nie | Liczba dowiązań twardych. |
| `volume` | path | nie | Punkt montowania woluminu. |
| `file_id` | text | nie | Identyfikator pliku na woluminie. |
| `language` | text | nie | Język programowania lub znaczników. |
| `mime` | text | nie | Typ nośnika. |
| `binary` | bool | nie | Treść binarna. |
| `encoding` | text | nie | Wykryte kodowanie pliku tekstowego. |
| `line_ending` | enum(lf\|crlf\|cr\|mixed\|none) | nie | Zakończenia wierszy pliku tekstowego. |
| `bom` | bool | nie | Czy tekst zaczyna się od znacznika kolejności bajtów. |
| `lines` | int | nie | Liczba wierszy. |
| `hash` | text | nie | Skrót treści, na przykład `sha256:…`. |
| `etag` | text | nie | Znacznik encji treści w chwili odczytu. |
| `raw_name` | bytes | nie | Nazwa jako bajty, gdy nie jest poprawnym Unicode lub różni się od postaci NFC. |
| `real_path` | path | nie | Ścieżka po rozwiązaniu dowiązań. |
| `case_alias` | text | nie | Faktyczna pisownia na woluminie ignorującym wielkość liter. |
| `portability` | list<text> | nie | Problemy z przenośnością nazwy, na przykład `windows_reserved_name`. |
| `depth` | int | nie | Głębokość pod korzeniem wykazu. |
| `relative` | text | nie | Ścieżka względem korzenia wykazu. |
| `raw_relative` | bytes | nie | Natywne bajty `relative`, z separatorami `/`; obecne tylko wtedy, gdy nie jest poprawnym Unicode. |
| `total_size` | size | nie | Łączny rozmiar drzewa katalogów. |
| `file_count` | int | nie | Liczba plików w drzewie katalogów. |
| `rank` | number | nie | Ranga podobieństwa wyszukiwania rozmytego. |
| `match_count` | int | nie | Liczba dopasowań w pliku. |
| `xattrs` | map<text> | nie | Atrybuty rozszerzone. |
| `streams` | list<text> | nie | Alternatywne strumienie danych. |
| `flags` | list<text> | nie | Flagi pliku, takie jak `immutable`. |
| `details` | map<any> | nie | Szczegóły żądane przez `details:`, według rodzaju. |
| `error` | text | nie | Błąd, który uniemożliwił pełny odczyt wpisu. |

### content {#content}

Treść tekstowa pliku, zdekodowana do UTF-8.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik. |
| `raw_path` | bytes | nie | Natywne bajty `path`; obecne tylko wtedy, gdy ścieżka nie jest poprawnym Unicode. |
| `text` | text | tak | Tekst, ewentualnie ograniczony do zakresu. |
| `lines` | int | tak | Liczba wierszy w `text`. |
| `size` | size | tak | Rozmiar pliku w bajtach. |
| `etag` | text | tak | Znacznik encji treści. |
| `encoding` | text | tak | Kodowanie pliku. |
| `bom` | bool | tak | Czy plik zaczyna się od znacznika kolejności bajtów. |
| `line_ending` | enum(lf\|crlf\|cr\|mixed\|none) | tak | Zakończenia wierszy pliku. |
| `range` | range | nie | Zakres wierszy w `text`. |
| `ranges` | list<range> | nie | Zakresy wierszy, gdy zażądano kilku. |
| `total_lines` | int | nie | Łączna liczba wierszy pliku. |
| `unchanged` | bool | nie | Plik nie zmienił się od podanego znacznika encji. |
| `truncated` | bool | nie | Tekst został ucięty na limicie. |
| `binary` | bool | nie | Treść binarna. |
| `image` | object [`ContentImage`](#contentimage) | nie | Treść obrazu. |
| `page` | int | nie | Strona dokumentu, od 1. |
| `pages_total` | int | nie | Liczba stron dokumentu. |
| `cells` | list<record [`cell`](#cell)> | nie | Komórki notatnika. |
| `symbols` | list<record [`symbol`](#symbol)> | nie | Symbole widoku konspektu. |
| `since_missing` | bool | nie | Treść podana w `since:` nie była dostępna. |
| `diff` | list<record [`hunk`](#hunk)> | nie | Różnica względem treści podanej w `since:`. |
| `decompressed_from` | path | nie | Skompresowany plik, z którego odczytano tekst. |
| `view` | text | nie | Użyty widok, na przykład `outline`. |
| `revision` | text | nie | Pełny identyfikator rewizji repozytorium, z której odczytano treść. |

### line {#line}

Jeden wiersz pliku.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik. |
| `raw_path` | bytes | nie | Natywne bajty `path`; obecne tylko wtedy, gdy ścieżka nie jest poprawnym Unicode. |
| `no` | int | tak | Numer wiersza, od 1. |
| `text` | text | tak | Tekst bez zakończenia wiersza. |
| `line_ending` | enum(lf\|crlf\|cr\|mixed\|none) | tak | Zakończenie wiersza. |
| `etag` | text | tak | Znacznik encji pliku. |
| `start` | int | nie | Przesunięcie bajtowe wiersza od początku pliku. |
| `end` | int | nie | Przesunięcie bajtowe za wierszem, bez zakończenia wiersza. |
| `range_index` | int | nie | Indeks żądanego zakresu, do którego należy wiersz. |
| `encoding` | text | nie | Kodowanie pliku. |

### match {#match}

Dopasowanie wyszukiwania.

`start` i `end` są przesunięciami bajtowymi od początku pliku; `column` liczy wartości skalarne Unicode od 1.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik. |
| `raw_path` | bytes | nie | Natywne bajty `path`; obecne tylko wtedy, gdy ścieżka nie jest poprawnym Unicode. |
| `no` | int | tak | Numer wiersza, od 1. |
| `column` | int | tak | Kolumna dopasowania, od 1. |
| `start` | int | tak | Przesunięcie bajtowe dopasowania od początku pliku. |
| `end` | int | tak | Przesunięcie bajtowe za dopasowaniem. |
| `text` | text | tak | Dopasowany tekst. |
| `line` | text | tak | Cały wiersz. |
| `etag` | text | tak | Znacznik encji pliku. |
| `groups` | list<text> | nie | Grupy przechwytujące według numeru; indeks 0 to całe dopasowanie, a `null` oznacza grupę, która nie została dopasowana. |
| `named` | map<text> | nie | Nazwane grupy przechwytujące. |
| `before` | list<text> | nie | Wiersze przed dopasowaniem. |
| `after` | list<text> | nie | Wiersze po dopasowaniu. |
| `count` | int | nie | Liczba dopasowań w pliku (`output: counts`). |
| `pattern` | int | nie | Indeks wzorca na liście wzorców. |
| `container` | object [`MatchContainer`](#matchcontainer) | nie | Otaczający symbol. |
| `regex_engine` | enum(linear\|backtracking) | nie | Silnik, który ocenił wzorzec. |
| `end_line` | int | nie | Ostatni wiersz dopasowania wielowierszowego. |
| `end_column` | int | nie | Kolumna za dopasowaniem w ostatnim wierszu. |
| `summarized` | bool | nie | Tylko policzone, niepokazane w całości. |

### item {#item}

Wartość pod ścieżką klucza w dokumencie strukturalnym.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik. |
| `format` | text | tak | Format dokumentu, na przykład `json`. |
| `key` | text | tak | Ścieżka klucza w postaci kanonicznej. |
| `value` | any | tak | Wartość. |
| `value_type` | enum(text\|int\|number\|bool\|null\|list\|map\|date\|bytes) | tak | Typ wartości w dokumencie. |
| `etag` | text | tak | Znacznik encji pliku. |
| `line` | int | nie | Wiersz, od 1. |
| `column` | int | nie | Kolumna, od 1. |
| `end_line` | int | nie | Ostatni wiersz. |
| `start` | int | nie | Przesunięcie bajtowe wartości od początku pliku. |
| `end` | int | nie | Przesunięcie bajtowe za wartością. |
| `comment` | text | nie | Komentarz dołączony do wartości. |
| `document` | int | nie | Indeks dokumentu w pliku z wieloma dokumentami. |
| `anchor` | text | nie | Kotwica wartości. |
| `alias_of` | text | nie | Kotwica, której aliasem jest ta wartość. |
| `tag` | text | nie | Tag wartości. |

### row {#row}

Wiersz tabeli, pliku CSV lub zapytania.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik. |
| `no` | int | tak | Numer wiersza, od 1. |
| `cells` | map<any> | tak | Komórki według nazwy kolumny. |
| `table` | text | nie | Tabela lub instrukcja, z której pochodzi wiersz. |

### node {#node}

Węzeł drzewa składni. Rodzaj jest zarezerwowany: katalog wymienia `node` wśród rodzajów wejściowych niektórych poleceń, ale żadne polecenie tego wydania nie tworzy rekordów `node`, więc potok nie może dostarczyć takiego rekordu.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik. |
| `language` | text | tak | Język. |
| `kind` | text | tak | Rodzaj węzła w gramatyce. |
| `start` | int | tak | Przesunięcie bajtowe od początku pliku. |
| `end` | int | tak | Przesunięcie bajtowe za węzłem. |
| `line` | int | tak | Pierwszy wiersz, od 1. |
| `column` | int | tak | Pierwsza kolumna, od 1. |
| `end_line` | int | tak | Ostatni wiersz. |
| `text` | text | tak | Tekst węzła. |
| `etag` | text | tak | Znacznik encji pliku. |
| `end_column` | int | nie | Kolumna za węzłem w ostatnim wierszu. |
| `captures` | map<text> | nie | Przechwycone metazmienne wzorca. |

### symbol {#symbol}

Symbol kodu źródłowego.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Nazwa. |
| `kind` | enum(function\|method\|class\|struct\|interface\|enum\|variable\|constant\|field\|module\|type\|macro\|other) | tak | Rodzaj. |
| `path` | path | tak | Plik. |
| `line` | int | tak | Pierwszy wiersz, od 1. |
| `column` | int | tak | Kolumna, od 1. |
| `end_line` | int | tak | Ostatni wiersz. |
| `language` | text | tak | Język. |
| `source` | enum(lsp\|syntax) | tak | Źródło informacji. |
| `etag` | text | tak | Znacznik encji pliku. |
| `end_column` | int | nie | Kolumna za symbolem w ostatnim wierszu. |
| `container` | text | nie | Otaczający symbol. |
| `signature` | text | nie | Sygnatura. |
| `text` | text | nie | Tekst definicji. |
| `exported` | bool | nie | Wyeksportowany z modułu. |
| `references` | int | nie | Liczba odwołań. |

### reference {#reference}

Odwołanie do symbolu.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Nazwa symbolu. |
| `path` | path | tak | Plik. |
| `line` | int | tak | Wiersz, od 1. |
| `column` | int | tak | Kolumna, od 1. |
| `role` | enum(definition\|reference\|import\|test\|unknown) | tak | Rola. |
| `source` | enum(lsp\|syntax) | tak | Źródło informacji. |
| `etag` | text | tak | Znacznik encji pliku. |
| `end_column` | int | nie | Kolumna za odwołaniem. |
| `text` | text | nie | Tekst wiersza. |
| `container` | text | nie | Otaczający symbol. |

### diagnostic {#diagnostic}

Diagnostyka kompilatora, lintera lub serwera języka.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik. |
| `line` | int | tak | Wiersz, od 1. |
| `column` | int | tak | Kolumna, od 1. |
| `severity` | enum(error\|warning\|info\|hint) | tak | Waga. |
| `message` | text | tak | Komunikat. |
| `source` | text | tak | Narzędzie lub serwer, który ją zgłosił. |
| `code` | text | nie | Kod diagnostyki. |
| `end_line` | int | nie | Ostatni wiersz. |
| `end_column` | int | nie | Kolumna za fragmentem w ostatnim wierszu. |
| `language` | text | nie | Język. |
| `related` | list<object [`RelatedLocation`](#relatedlocation)> | nie | Powiązane pozycje. |

### cell {#cell}

Komórka notatnika.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik notatnika. |
| `index` | int | tak | Pozycja komórki, od 0. |
| `cell_type` | enum(code\|markdown\|raw) | tak | Typ komórki. |
| `source` | text | tak | Źródło komórki. |
| `etag` | text | tak | Znacznik encji pliku notatnika. |
| `id` | text | nie | Identyfikator komórki. |
| `outputs` | list<any> | nie | Wyjścia w postaci zapisanej w notatniku. |
| `outputs_summary` | text | nie | Liczba i rodzaje wyjść, bez ich treści. |
| `execution_count` | int | nie | Licznik wykonań. |
| `metadata` | map<any> | nie | Metadane komórki. |

### hunk {#hunk}

Fragment (hunk) różnicy w formacie unified.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik po zmianie. |
| `raw_path` | bytes | nie | Natywne bajty `path`; obecne tylko wtedy, gdy ścieżka nie jest poprawnym Unicode. |
| `old_start` | int | tak | Pierwszy wiersz przed zmianą, od 1. |
| `old_len` | int | tak | Liczba wierszy przed zmianą. |
| `new_start` | int | tak | Pierwszy wiersz po zmianie, od 1. |
| `new_len` | int | tak | Liczba wierszy po zmianie. |
| `lines` | list<text> | tak | Wiersze z prefiksami `+`, `-` i spacją oraz wiersz znacznika `\ No newline at end of file`. |
| `id` | text | nie | Stabilny identyfikator, `path#number`. |
| `header` | text | nie | Nagłówek sekcji po `@@`. |
| `old_path` | path | nie | Plik przed zmianą nazwy. |
| `raw_old_path` | bytes | nie | Natywne bajty `old_path`; obecne tylko wtedy, gdy nie jest poprawnym Unicode. |
| `status` | enum(modified\|added\|deleted\|renamed\|binary\|copied) | nie | Stan pliku. |
| `binary` | bool | nie | Treść binarna. |
| `old_mode` | text | nie | Tryb przed zmianą. |
| `new_mode` | text | nie | Tryb po zmianie. |
| `ignore_whitespace` | bool | nie | Białe znaki zostały zignorowane. |
| `fingerprint` | text | nie | Pierwsze sześć cyfr szesnastkowych skrótu SHA-256 wierszy fragmentu; część identyfikatora `path#number:fingerprint`. |
| `staged` | bool | nie | Fragment znajduje się w indeksie, a nie w drzewie roboczym. |
| `untracked` | bool | nie | Fragment dodaje plik nieśledzony. |
| `unit` | enum(lines\|words) | nie | Jednostka różnicy. |
| `moved` | list<object [`MovedBlock`](#movedblock)> | nie | Bloki wierszy przeniesione w obrębie różnicy. |

#### MovedBlock {#movedblock}

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `from_path` | path | tak | Plik, z którego usunięto wiersze. |
| `from_line` | int | tak | Pierwszy usunięty wiersz, od 1. |
| `to_path` | path | tak | Plik, do którego dodano wiersze. |
| `to_line` | int | tak | Pierwszy dodany wiersz, od 1. |
| `lines` | int | tak | Liczba wierszy. |

### change {#change}

Zmiana jednego pliku w ramach skutku.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik. |
| `raw_path` | bytes | nie | Natywne bajty `path`; obecne tylko wtedy, gdy ścieżka nie jest poprawnym Unicode. |
| `action` | enum(create\|modify\|delete\|rename\|attributes\|link\|truncate\|state) | tak | Akcja. |
| `from` | path | nie | Poprzednia ścieżka przy zmianie nazwy. |
| `raw_from` | bytes | nie | Natywne bajty `from`; obecne tylko wtedy, gdy nie jest poprawnym Unicode. |
| `to` | path | nie | Nowa ścieżka przeniesienia lub kopii. |
| `raw_to` | bytes | nie | Natywne bajty `to`; obecne tylko wtedy, gdy nie jest poprawnym Unicode. |
| `size_before` | size | nie | Rozmiar przed zmianą. |
| `size_after` | size | nie | Rozmiar po zmianie. |
| `hash_before` | text | nie | Skrót przed zmianą. |
| `hash_after` | text | nie | Skrót po zmianie. |
| `added` | int | nie | Dodane wiersze. |
| `removed` | int | nie | Usunięte wiersze. |
| `replacements` | int | nie | Wykonane zamiany. |
| `rows_changed` | int | nie | Wiersze wstawione, zaktualizowane lub usunięte przez instrukcje SQL. |
| `flags` | list<text> | nie | Uwagi o zmianie, na przykład `crlf_preserved`. |
| `similarity` | int | nie | Podobieństwo przy zmianie nazwy w procentach. |
| `encoding_before` | text | nie | Kodowanie przed zmianą. |
| `encoding_after` | text | nie | Kodowanie po zmianie. |
| `line_ending_before` | enum(lf\|crlf\|cr\|mixed\|none) | nie | Zakończenia wierszy przed zmianą. |
| `line_ending_after` | enum(lf\|crlf\|cr\|mixed\|none) | nie | Zakończenia wierszy po zmianie. |
| `bom_before` | bool | nie | Znacznik kolejności bajtów przed zmianą. |
| `bom_after` | bool | nie | Znacznik kolejności bajtów po zmianie. |
| `link_target` | path | nie | Cel utworzonego dowiązania. |
| `raw_link_target` | bytes | nie | Natywne bajty `link_target`; obecne tylko wtedy, gdy nie jest poprawnym Unicode. |
| `store` | text | nie | Sposób przechowania poprzedniej treści. |
| `irreversible` | enum (przyczyna z raportu skutku) | nie | Przyczyna, dla której zmiany nie można cofnąć, na przykład `no_space_for_copy`. |
| `diff` | list<record [`hunk`](#hunk)> | nie | Różnica treści. |
| `before` | text | nie | Stan przed zmianą akcji `state`, na przykład `running`. |
| `after` | text | nie | Stan po zmianie akcji `state`. |

### exec {#exec}

Wynik zakończonego uruchomienia programu.

`exit_code` wynosi `-1`, gdy proces zakończył się sygnałem lub limitem; `signal` i `killed_by` wskazują, którym.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `command` | list<text> | tak | Argumenty w podanej postaci. |
| `program` | path | tak | Program po rozwiązaniu w ścieżce wyszukiwania. |
| `cwd` | path | tak | Katalog roboczy. |
| `pid` | int | tak | Identyfikator procesu. |
| `exit_code` | int | tak | Kod wyjścia. |
| `timed_out` | bool | tak | Zakończony przez limit czasu. |
| `stalled` | bool | tak | Zakończony przez limit bezczynności. |
| `started` | instant | tak | Czas rozpoczęcia. |
| `elapsed` | duration | tak | Czas, który upłynął. |
| `stdout` | text | tak | Standardowe wyjście po filtrach. |
| `stderr` | text | tak | Standardowe wyjście błędów po filtrach. |
| `stdout_bytes` | int | tak | Bajty standardowego wyjścia. |
| `stderr_bytes` | int | tak | Bajty standardowego wyjścia błędów. |
| `stdout_lines` | int | tak | Wiersze standardowego wyjścia. |
| `stderr_lines` | int | tak | Wiersze standardowego wyjścia błędów. |
| `decoded_as` | text | tak | Kodowanie, z którego zdekodowano wyjście, na przykład `utf-8`. |
| `changes` | list<record [`change`](#change)> | tak | Pliki zmienione przez uruchomienie. |
| `tracking` | enum(none\|scan\|events) | tak | Sposób wykrywania zmian. |
| `watch` | list<path> | tak | Katalogi obserwowane pod kątem zmian. |
| `launcher` | enum(direct\|cmd) | nie | Sposób uruchomienia programu. |
| `signal` | enum(terminate\|kill\|interrupt\|hangup\|quit\|abort\|segv\|bus\|fpe\|ill\|pipe\|other) | nie | Sygnał, który zakończył proces. |
| `signal_number` | int | nie | Numer tego sygnału. |
| `killed_by` | enum(timeout\|idle\|limit\|stop\|cancel) | nie | Kto zakończył proces. |
| `limit_hit` | enum(cpu\|memory) | nie | Osiągnięty limit zasobu. |
| `finished` | instant | nie | Czas zakończenia. |
| `cpu_time` | duration | nie | Czas procesora grupy procesów. |
| `max_rss` | size | nie | Szczytowa pamięć grupy procesów. |
| `shown` | object [`ExecShown`](#execshown) | nie | Co z wyjścia zostało pokazane. |
| `output_file` | path | nie | Plik ze scalonym wyjściem. |
| `stdout_file` | path | nie | Plik ze standardowym wyjściem. |
| `stderr_file` | path | nie | Plik ze standardowym wyjściem błędów. |
| `output_cursor` | text | nie | Kursor do odczytu dalszego wyjścia. |
| `output_sha256` | text | nie | Skrót kompletnego wyjścia. |
| `summary_lines` | list<text> | nie | Wiersze dopasowane przez `summary:`. |
| `progress` | object [`Progress`](#progress) | nie | Ostatni postęp. |
| `preimage` | enum(none\|clone\|copy\|partial) | nie | Poprzednia treść zachowana przed uruchomieniem. |
| `preimage_entries` | int | nie | Liczba zachowanych plików. |
| `preimage_bytes` | size | nie | Zachowane bajty. |
| `external_effect` | list<text> | nie | Skutki poza systemem plików rozpoznane w argumentach. |
| `batches` | list<object [`Batch`](#batch)> | nie | Partie uruchomienia na strumieniu rekordów. |
| `failed_batches` | int | nie | Liczba nieudanych partii. |
| `env_keys` | list<text> | nie | Nazwy jawnie ustawionych zmiennych środowiskowych. |
| `job` | text | nie | Identyfikator zadania, gdy uruchomienie było kontynuowane w tle. |
| `diagnostics` | object [`ExecDiagnostics`](#execdiagnostics) | nie | Błędy i ostrzeżenia rozpoznanego narzędzia. |
| `digest` | text | nie | Jeden wiersz o udanym uruchomieniu: kod wyjścia, czas, wiersze i bajty. |

### job {#job}

Zadanie w tle.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `id` | text | tak | Identyfikator, na przykład `j7`. |
| `command` | list<text> | tak | Argumenty. |
| `cwd` | path | tak | Katalog roboczy. |
| `pid` | int | tak | Identyfikator procesu. |
| `state` | enum(running\|suspended\|exited\|failed\|stopped\|timed_out\|stalled\|limit\|lost) | tak | Stan. |
| `started` | instant | tak | Czas rozpoczęcia. |
| `elapsed` | duration | tak | Czas od rozpoczęcia lub do zakończenia. |
| `label` | text | nie | Etykieta. |
| `description` | text | nie | Opis. |
| `program` | path | nie | Program po rozwiązaniu. |
| `exit_code` | int | nie | Kod wyjścia po zakończeniu. |
| `signal` | enum(terminate\|kill\|interrupt\|hangup\|quit\|abort\|segv\|bus\|fpe\|ill\|pipe\|other) | nie | Sygnał, który zakończył proces. |
| `killed_by` | enum(timeout\|idle\|limit\|stop\|cancel) | nie | Kto zakończył proces. |
| `finished` | instant | nie | Czas zakończenia. |
| `progress` | object [`Progress`](#progress) | nie | Ostatni postęp. |
| `stdout` | text | nie | Standardowe wyjście, gdy zażądano. |
| `stderr` | text | nie | Standardowe wyjście błędów, gdy zażądano. |
| `stdout_bytes` | int | nie | Bajty standardowego wyjścia. |
| `stderr_bytes` | int | nie | Bajty standardowego wyjścia błędów. |
| `stdout_lines` | int | nie | Wiersze standardowego wyjścia. |
| `stderr_lines` | int | nie | Wiersze standardowego wyjścia błędów. |
| `output_file` | path | nie | Plik ze scalonym wyjściem. |
| `stdout_file` | path | nie | Plik ze standardowym wyjściem. |
| `stderr_file` | path | nie | Plik ze standardowym wyjściem błędów. |
| `output_cursor` | text | nie | Kursor do odczytu dalszego wyjścia. |
| `retry_of` | text | nie | Zadanie, które to zadanie powtarza. |
| `op` | int | nie | Numer operacji dziennika dla zadania. |
| `changes` | list<record [`change`](#change)> | nie | Zmienione pliki, po zakończeniu. |
| `session` | text | nie | Sesja, która uruchomiła zadanie. |
| `suspended_total` | duration | nie | Łączny czas zawieszenia. |

### process {#process}

Proces systemu operacyjnego.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `pid` | int | tak | Identyfikator procesu. |
| `parent_pid` | int | tak | Identyfikator procesu nadrzędnego. |
| `name` | text | tak | Nazwa. |
| `command` | list<text> | tak | Argumenty. |
| `user` | text | tak | Nazwa konta. |
| `memory` | size | tak | Pamięć rezydentna. |
| `started` | instant | tak | Czas rozpoczęcia. |
| `state` | enum(running\|sleeping\|stopped\|zombie\|idle\|other) | tak | Stan. |
| `executable` | path | nie | Plik wykonywalny. |
| `uid` | text | nie | Identyfikator konta: UID lub SID. |
| `cpu` | number | nie | Użycie procesora w procentach jednego rdzenia. |
| `threads` | int | nie | Liczba wątków. |
| `cwd` | path | nie | Katalog roboczy. |
| `children` | list<int> | nie | Procesy potomne. |
| `depth` | int | nie | Głębokość w drzewie procesów. |
| `os_raw` | map<any> | nie | Surowe atrybuty systemu, poza gwarancją stabilności. |

### port {#port}

Port sieciowy.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `protocol` | enum(tcp\|udp) | tak | Protokół. |
| `local` | text | tak | Adres lokalny i port, `address:port`. |
| `address` | text | tak | Adres lokalny. |
| `port` | int | tak | Port lokalny. |
| `state` | enum(listen\|established\|time_wait\|close_wait\|syn_sent\|syn_received\|fin_wait\|closing\|last_ack\|closed\|none) | tak | Stan. |
| `remote` | text | nie | Adres zdalny i port. |
| `remote_address` | text | nie | Adres zdalny. |
| `remote_port` | int | nie | Port zdalny. |
| `pid` | int | nie | Proces będący właścicielem. |
| `process` | text | nie | Nazwa procesu będącego właścicielem. |
| `family` | enum(ipv4\|ipv6) | nie | Rodzina adresów. |

### disk {#disk}

Zamontowany dysk.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `mount` | path | tak | Punkt montowania. |
| `device` | text | tak | Urządzenie. |
| `filesystem` | text | tak | System plików. |
| `total` | size | tak | Rozmiar całkowity. |
| `free` | size | tak | Wolny rozmiar dostępny dla konta. |
| `used` | size | tak | Zajęty rozmiar. |
| `type` | enum(fixed\|removable\|network\|ram\|other) | tak | Typ. |
| `case_sensitive` | bool | tak | Nazwy różnią się wielkością liter. |
| `readonly` | bool | tak | Zamontowany tylko do odczytu. |
| `label` | text | nie | Etykieta. |
| `inodes_total` | int | nie | Łączna liczba i-węzłów. |
| `inodes_free` | int | nie | Wolne i-węzły. |
| `options` | list<text> | nie | Opcje montowania. |
| `os_raw` | map<any> | nie | Surowe atrybuty systemu, poza gwarancją stabilności. |

### host {#host}

Opis urządzenia.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `os` | enum(linux\|macos\|windows) | tak | System operacyjny. |
| `os_version` | text | tak | Wersja systemu operacyjnego. |
| `architecture` | text | tak | Architektura procesora, na przykład `x86_64`. |
| `cpus` | int | tak | Procesory logiczne. |
| `memory_total` | size | tak | Pamięć całkowita. |
| `hostname` | text | tak | Nazwa hosta. |
| `user` | text | tak | Bieżące konto. |
| `home` | path | tak | Katalog domowy. |
| `temp` | path | tak | Katalog plików tymczasowych. |
| `timezone` | text | tak | Lokalna strefa czasowa. |
| `path_style` | text | tak | Styl ścieżek: `posix` lub `windows`. |
| `nurt_version` | text | tak | Wersja tej implementacji. |
| `features` | list<text> | tak | Dostępne funkcje, na przykład `reflink` i `journal`. |
| `kernel` | text | nie | Wersja jądra. |
| `memory_free` | size | nie | Pamięć wolna. |
| `uid` | text | nie | Identyfikator konta. |
| `groups` | list<text> | nie | Grupy konta. |
| `locale` | text | nie | Ustawienia regionalne. |
| `uptime` | duration | nie | Czas od uruchomienia systemu. |
| `boot_time` | instant | nie | Czas uruchomienia systemu. |
| `programs` | list<record [`program_location`](#program_location)> | nie | Programy znalezione w ścieżce wyszukiwania. |
| `virtualization` | text | nie | Wirtualizacja, gdy wykryto. |
| `service` | enum(running\|stopped\|not_installed) | nie | Stan usługi w tle danej instancji. |

### environment_variable {#environmentvariable}

Zmienna środowiskowa; wartość nigdy nie jest maskowana.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Nazwa. |
| `value` | text | tak | Wartość. |
| `scope` | enum(process\|user\|machine) | tak | Zakres. |

### service {#service}

Usługa systemu operacyjnego.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Nazwa. |
| `state` | enum(running\|stopped\|starting\|stopping\|failed\|paused\|unknown) | tak | Stan. |
| `start_type` | enum(automatic\|manual\|disabled\|on_demand\|static\|unknown) | tak | Typ uruchamiania. |
| `manager` | enum(systemd\|windows\|launchd) | tak | Menedżer usług. |
| `display_name` | text | nie | Nazwa wyświetlana. |
| `state_raw` | text | nie | Surowy stan zgłoszony przez menedżera. |
| `pid` | int | nie | Proces główny. |
| `description` | text | nie | Opis. |
| `scope` | enum(system\|user) | nie | Zakres. |
| `path` | path | nie | Plik jednostki, lista właściwości lub program usługi. |
| `account` | text | nie | Konto, z którego działa usługa. |
| `os_raw` | map<any> | nie | Surowe atrybuty systemu, poza gwarancją stabilności. |

### package {#package}

Pakiet zainstalowany lub dostępny.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Nazwa. |
| `version` | text | tak | Wersja zainstalowana lub oferowana. |
| `manager` | enum(apt\|dnf\|pacman\|zypper\|brew\|winget\|choco\|scoop\|dpkg\|rpm\|other) | tak | Menedżer pakietów. |
| `installed` | bool | tak | Zainstalowany. |
| `available_version` | text | nie | Dostępna nowsza wersja. |
| `description` | text | nie | Opis. |
| `architecture` | text | nie | Architektura. |
| `size` | size | nie | Rozmiar po instalacji. |
| `source` | text | nie | Repozytorium. |
| `files_count` | int | nie | Liczba plików. |

### account {#account}

Konto użytkownika lub grupy.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Nazwa. |
| `kind` | enum(user\|group) | tak | Rodzaj. |
| `id` | text | tak | Identyfikator: UID, GID lub SID. |
| `home` | path | nie | Katalog domowy. |
| `shell` | path | nie | Powłoka logowania. |
| `groups` | list<text> | nie | Grupy użytkownika lub członkowie grupy. |
| `disabled` | bool | nie | Wyłączone. |
| `full_name` | text | nie | Pełna nazwa. |
| `system` | bool | nie | Konto systemowe. |
| `last_login` | instant | nie | Ostatnie logowanie. |
| `os_raw` | map<any> | nie | Surowe atrybuty systemu, poza gwarancją stabilności. |

### log_event {#logevent}

Zdarzenie dziennika systemowego.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `time` | instant | tak | Czas. |
| `source` | text | tak | Program, dostawca lub podsystem, który zapisał zdarzenie. |
| `level` | enum(critical\|error\|warning\|info\|debug) | tak | Poziom. |
| `message` | text | tak | Komunikat. |
| `id` | text | nie | Identyfikator zdarzenia. |
| `host` | text | nie | Nazwa hosta. |
| `unit` | text | nie | Jednostka, kanał lub kategoria. |
| `pid` | int | nie | Identyfikator procesu. |
| `boot_id` | text | nie | Identyfikator uruchomienia systemu. |
| `cursor` | text | nie | Pozycja do stronicowania i wznawiania. |
| `fields` | map<text> | nie | Pozostałe pola. |

### commit {#commit}

Commit repozytorium.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `id` | text | tak | Pełny identyfikator obiektu. |
| `short_id` | text | tak | Skrócony identyfikator. |
| `author` | text | tak | Nazwa autora. |
| `time` | instant | tak | Czas autora. |
| `message` | text | tak | Pełny komunikat. |
| `subject` | text | tak | Pierwszy wiersz komunikatu. |
| `parents` | list<text> | tak | Identyfikatory rodziców. |
| `author_email` | text | nie | Adres e-mail autora. |
| `committer` | text | nie | Nazwa osoby zatwierdzającej. |
| `committer_email` | text | nie | Adres e-mail osoby zatwierdzającej. |
| `commit_time` | instant | nie | Czas commitu. |
| `refs` | list<text> | nie | Nazwy referencji wskazujących na commit. |
| `signature` | object [`CommitSignature`](#commitsignature) | nie | Podpis. |
| `stats` | object [`CommitStats`](#commitstats) | nie | Statystyki zmian. |
| `trailers` | map<text> | nie | Trailery komunikatu. |
| `tree` | text | nie | Identyfikator drzewa. |
| `files` | list<object [`CommitFile`](#commitfile)> | nie | Zmienione pliki. |
| `branch` | text | nie | Gałąź, która otrzymała commit, w wyniku `git.commit`. |

### git_status {#gitstatus}

Stan ścieżki w repozytorium.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | text | tak | Ścieżka w repozytorium. |
| `raw_path` | bytes | nie | Bajty `path`; obecne tylko wtedy, gdy nie jest poprawnym Unicode. |
| `index` | enum(unmodified\|modified\|added\|deleted\|renamed\|copied\|type_changed\|unmerged\|untracked\|ignored) | tak | Stan w indeksie. |
| `worktree` | enum(unmodified\|modified\|added\|deleted\|renamed\|copied\|type_changed\|unmerged\|untracked\|ignored) | tak | Stan w drzewie roboczym. |
| `kind` | enum(tracked\|untracked\|ignored\|conflict) | tak | Rodzaj wpisu. |
| `repo` | path | tak | Korzeń repozytorium. |
| `renamed_from` | text | nie | Ścieżka przed zmianą nazwy. |
| `raw_renamed_from` | bytes | nie | Bajty `renamed_from`; obecne tylko wtedy, gdy nie jest poprawnym Unicode. |
| `conflict` | enum(both_modified\|both_added\|both_deleted\|added_by_us\|added_by_them\|deleted_by_us\|deleted_by_them\|rename_rename\|rename_delete\|directory_file\|binary\|submodule) | nie | Rodzaj konfliktu. |
| `submodule` | object [`Submodule`](#submodule) | nie | Stan podmodułu. |
| `mode_before` | text | nie | Tryb przed zmianą. |
| `mode_after` | text | nie | Tryb po zmianie. |
| `added` | int | nie | Dodane wiersze. |
| `removed` | int | nie | Usunięte wiersze. |
| `similarity` | int | nie | Podobieństwo przy zmianie nazwy w procentach. |

### branch {#branch}

Gałąź repozytorium.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Nazwa. |
| `current` | bool | tak | Wyewidencjonowana w tym drzewie roboczym. |
| `commit` | text | tak | Identyfikator commitu. |
| `remote` | bool | tak | Gałąź śledząca gałąź zdalną. |
| `upstream` | text | nie | Gałąź nadrzędna (upstream). |
| `ahead` | int | nie | Commity przed gałęzią nadrzędną. |
| `behind` | int | nie | Commity za gałęzią nadrzędną. |
| `upstream_gone` | bool | nie | Gałąź nadrzędna już nie istnieje. |
| `worktree` | path | nie | Drzewo robocze, w którym gałąź jest wyewidencjonowana. |
| `subject` | text | nie | Temat ostatniego commitu. |
| `time` | instant | nie | Czas ostatniego commitu. |

### schedule {#schedule}

Zaplanowane zadanie.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `id` | text | tak | Identyfikator, na przykład `s3`. |
| `trigger` | object [`Trigger`](#trigger) | tak | Wyzwalacz. |
| `next_run` | instant | tak | Następne uruchomienie. |
| `active` | bool | tak | Zadanie jest uruchamiane. |
| `action` | text | tak | Akcja: kanoniczny tekst programu. |
| `label` | text | nie | Etykieta. |
| `last_run` | instant | nie | Ostatnie uruchomienie. |
| `last_result` | enum(ok\|failed\|timed_out\|skipped_overlap\|missed\|cancelled) | nie | Wynik ostatniego uruchomienia. |
| `state` | enum(active\|paused\|completed\|expired) | nie | Stan. |
| `runs` | int | nie | Dotychczasowe uruchomienia. |
| `max_runs` | int | nie | Maksymalna liczba uruchomień. |
| `expires` | instant | nie | Czas wygaśnięcia. |
| `on_missed` | text | nie | Obsługa uruchomień pominiętych, gdy usługa nie działała. |
| `misfire` | duration | nie | Tolerancja dla spóźnionego startu. |
| `overlap` | text | nie | Obsługa uruchomienia, które startuje, gdy poprzednie jeszcze trwa. |
| `timeout` | duration | nie | Limit czasu uruchomienia. |
| `created` | instant | nie | Czas utworzenia. |
| `session` | text | nie | Sesja, która utworzyła zadanie. |
| `action_kind` | text | nie | Rodzaj akcji, na przykład `pipeline`. |
| `reason` | text | nie | Dlaczego zadanie nie jest aktywne, na przykład `service_not_running`. |

### operation {#operation}

Wpis dziennika operacji.

Rekord nie pokazuje czasu wygaśnięcia ani niczego, z czego można by go wyprowadzić.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `id` | int | tak | Numer operacji, jedna sekwencja na konto, od 1. |
| `time` | instant | tak | Czas zatwierdzenia. |
| `kind` | text | tak | Polecenie pod nazwą z katalogu, na przykład `replace` lub `git.stash`. |
| `action` | text | nie | Akcja zmieniająca polecenia z wieloma akcjami, na przykład `push`. |
| `summary` | text | tak | Jednowierszowe podsumowanie skutku. |
| `state` | enum(available\|partial\|expired\|undone) | tak | Stan. |
| `files` | int | tak | Liczba elementów. |
| `bytes` | size | tak | Zmienione bajty logiczne. |
| `irreversible` | list<object (wpis nieodwracalny raportu skutku)> | tak | Skutki, których nie można cofnąć; pusta, gdy można cofnąć wszystko. |
| `bulk` | bool | tak | Wiele celów w jednej operacji. |
| `session` | text | nie | Sesja wywołania. |
| `undo_of` | list<int> | tak | Operacje, które ta operacja cofnęła; pusta dla innych operacji. |
| `redo_of` | int | nie | Operacja cofnięcia, którą ta operacja ponowiła. |
| `undo_scope` | enum(op\|to\|file\|paths) | nie | Zakres operacji cofnięcia. |
| `ast_hash` | text | tak | Skrót kanonicznego, powiązanego programu, `sha256:` i 64 cyfry szesnastkowe. |
| `manifest_sha256` | text | tak | Skrót manifestu wpisu. |
| `chain` | text | nie | Skrót manifestu poprzedniego wpisu konta; nieobecny dla pierwszego wpisu. |
| `pinned` | bool | tak | Chroniony przed wygaśnięciem według liczby kroków, wieku i rozmiaru, ale nie przed potrzebą miejsca. |
| `items` | list<object [`OperationItem`](#operationitem)> | nie | Elementy wpisu; obecne tylko wtedy, gdy zażądano szczegółów, i puste dla wygasłego wpisu. |

#### OperationItem {#operationitem}

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Cel elementu. |
| `raw_path` | bytes | nie | Natywne bajty `path`; obecne tylko wtedy, gdy ścieżka nie jest poprawnym Unicode. |
| `action` | text | tak | Akcja na celu, nazwa akcji celu dziennika, na przykład `create`, `modify` lub `rename`. |
| `mode` | text | tak | Tryb zachowania poprzedniej treści, na przykład `clone` lub `none`. |
| `before_hash` | text | nie | Skrót treści przed operacją; nieobecny, gdy treści nie było. |
| `after_hash` | text | nie | Skrót treści po operacji; nieobecny, gdy treści nie ma. |
| `undone` | bool | tak | Element został cofnięty. |
| `released` | bool | tak | Zachowana poprzednia treść elementu została zwolniona. |

### response {#response}

Odpowiedź otrzymana na żądanie HTTP.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `url` | text | tak | Żądany URL. |
| `status` | int | tak | Kod statusu. |
| `headers` | map<any> | tak | Nagłówki według nazwy pisanej małymi literami; powtórzone nagłówki połączone, `set-cookie` jako lista. |
| `body` | text | tak | Treść zdekodowana do tekstu. |
| `bytes` | size | tak | Otrzymane bajty. |
| `elapsed` | duration | tak | Czas, który upłynął. |
| `final_url` | text | tak | URL po przekierowaniach. |
| `saved` | path | nie | Plik, do którego zapisano treść. |
| `content_type` | text | nie | Typ nośnika. |
| `charset` | text | nie | Zestaw znaków. |
| `truncated` | bool | nie | Treść została ucięta. |
| `offset` | int | nie | Przesunięcie treści w znakach. |
| `total_chars` | int | nie | Łączna liczba znaków treści. |
| `redirects` | list<text> | nie | URL-e przekierowań. |
| `from_cache` | bool | nie | Zwrócona z pamięci podręcznej. |
| `unchanged` | bool | nie | Niezmieniona od podanego czasu lub tagu. |
| `etag` | text | nie | Znacznik encji odpowiedzi. |
| `last_modified` | instant | nie | Zgłoszona ostatnia modyfikacja. |
| `remote_address` | text | nie | Adres hosta zdalnego. |

### volume {#volume}

Wolumin widziany przez dziennik.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `mount` | path | tak | Punkt montowania. |
| `filesystem` | text | tak | System plików. |
| `total` | size | tak | Rozmiar całkowity. |
| `free` | size | tak | Wolny rozmiar. |
| `free_without_journal` | size | tak | Wolny rozmiar, gdyby dziennik nic nie zajmował. |
| `reclaimable` | size | tak | Rozmiar, który dziennik może zwolnić. |
| `reserve` | size | tak | Rezerwa utrzymywana jako wolna. |
| `soft` | size | tak | Miękki próg wolnego miejsca. |
| `journal_bytes` | size | tak | Bajty zajęte przez dziennik. |
| `journal` | bool | tak | Dziennik jest aktywny na woluminie. |
| `journal_reason` | text | nie | Dlaczego dziennik nie jest aktywny. |

### program_card {#programcard}

Opis programu.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Nazwa. |
| `summary` | text | tak | Jedno zdanie. |
| `source` | enum(path\|package\|app\|catalog) | tak | Główne źródło. |
| `installed` | bool | tak | Zainstalowany. |
| `guide_optional` | bool | tak | Instrukcje nie są wymagane przed użyciem. |
| `sources` | list<text> | tak | Źródła, z których złożono kartę. |
| `category` | text | nie | Kategoria. |
| `subcategory` | text | nie | Podkategoria. |
| `path` | path | nie | Ścieżka. |
| `version` | text | nie | Wersja. |
| `description` | text | nie | Opis. |
| `guide` | text | nie | Instrukcje użycia. |
| `guide_shared` | text | nie | Nazwa instrukcji współdzielonych przez kilka programów. |
| `package` | text | nie | Pakiet. |
| `homepage` | text | nie | Strona domowa. |
| `license` | text | nie | Licencja. |
| `aliases` | list<text> | nie | Inne nazwy, które wskazują na program. |
| `verified` | instant | nie | Czas ostatniej weryfikacji. |
| `tags` | list<text> | nie | Tagi. |
| `platforms` | list<text> | nie | Systemy operacyjne. |
| `score` | number | nie | Trafność w wyszukiwaniu. |
| `kind` | enum(program\|app\|guide) | nie | Rodzaj karty. |
| `category_source` | enum(catalog\|desktop\|package\|man\|inferred) | nie | Pochodzenie kategorii. |
| `has_guide` | bool | nie | Program ma instrukcje użycia. |
| `guide_required` | bool | nie | Instrukcje muszą zostać przeczytane przed użyciem. |
| `guide_name` | text | nie | Nazwa instrukcji użycia. |
| `guide_shared_with` | list<text> | nie | Programy współdzielące instrukcje użycia. |
| `overview` | text | nie | Przegląd programu. |
| `alternatives` | list<text> | nie | Inne ścieżki programu w ścieżce wyszukiwania. |
| `catalog_path` | text | nie | Ścieżka wpisu w katalogu programów. |
| `manager` | text | nie | Menedżer pakietów danego pakietu. |
| `app_id` | text | nie | Identyfikator aplikacji. |
| `language` | text | nie | Język podsumowania. |
| `query` | text | nie | Zapytanie wyszukiwania, które znalazło kartę. |
| `instead` | text | nie | Program do użycia zamiast tego, o który zapytano. |
| `etag` | text | nie | Znacznik encji opisu. |
| `requested` | text | nie | Nazwa w postaci zażądanej, gdy różni się od nazwy karty. |
| `alias_kind` | enum(alias\|versioned\|companion\|symlink\|alternative\|shim\|app_alias) | nie | Jak zażądana nazwa odnosi się do karty. |
| `unchanged` | bool | nie | Opis jest niezmieniony od podanego znacznika encji. |
| `guide_sections` | list<text> | nie | Nagłówki sekcji instrukcji. |
| `guide_files` | list<text> | nie | Pliki instrukcji. |
| `guide_etag` | text | nie | Znacznik encji instrukcji. |
| `when_to_use` | list<text> | nie | Sytuacje, do których program jest przeznaczony. |
| `not_for` | list<map> | nie | Zadania, do których program nie jest przeznaczony, każde z polami `task` i `use`. |
| `example` | text | nie | Przykład użycia. |
| `pitfalls` | text | nie | Pułapki użycia. |
| `man` | text | nie | Wiersz strony podręcznika. |
| `help` | text | nie | Tekst pomocy programu. |
| `help_status` | enum(ok\|timeout\|refused\|failed\|empty) | nie | Wynik odczytu tekstu pomocy. |
| `executed` | list<text> | nie | Wiersz polecenia uruchomiony w celu odczytu tekstu pomocy. |

### archive_entry {#archiveentry}

Element archiwum.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `archive` | path | tak | Plik archiwum. |
| `name` | text | tak | Ostatni składnik ścieżki elementu. |
| `size` | size | tak | Rozmiar po rozpakowaniu. |
| `type` | enum(file\|directory\|link\|fifo\|device\|other) | tak | Typ. |
| `path` | text | nie | Pełna ścieżka wewnątrz archiwum. |
| `packed` | size | nie | Rozmiar w archiwum. |
| `modified` | instant | nie | Czas modyfikacji. |
| `mode` | text | nie | Bity uprawnień w zapisie ósemkowym. |
| `link_target` | text | nie | Cel dowiązania. |
| `compression` | text | nie | Metoda kompresji. |
| `encrypted` | bool | nie | Zaszyfrowany. |
| `crc32` | text | nie | Suma kontrolna CRC-32. |
| `format` | text | nie | Format archiwum. |

### test_run {#testrun}

Przebieg testów.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `id` | text | tak | Identyfikator, na przykład `run-3`. |
| `framework` | enum(bun\|vitest\|jest\|pytest\|go\|cargo\|npm\|custom) | tak | Framework. |
| `status` | enum(passed\|failed\|error\|timed_out\|no_tests) | tak | Wynik. |
| `passed` | int | tak | Testy zaliczone. |
| `failed` | int | tak | Testy niezaliczone. |
| `skipped` | int | tak | Testy pominięte. |
| `errors` | int | tak | Testy, których nie dało się uruchomić. |
| `duration` | duration | tak | Czas, który upłynął. |
| `command` | list<text> | tak | Argumenty polecenia. |
| `started` | instant | tak | Czas rozpoczęcia. |
| `cwd` | path | tak | Katalog roboczy. |
| `framework_version` | text | nie | Wersja frameworka. |
| `todo` | int | nie | Testy oznaczone jako jeszcze niezapisane. |
| `flaky` | int | nie | Testy zaliczone dopiero przy ponowieniu. |
| `fingerprint` | text | nie | Odcisk stanu kodu. |
| `targets` | list<text> | nie | Wybrane cele. |
| `filter` | text | nie | Filtr nazw. |
| `exit_code` | int | nie | Kod wyjścia frameworka. |
| `output_file` | path | nie | Plik z kompletnym wyjściem. |
| `retry_of` | text | nie | Przebieg, który ten przebieg powtarza. |
| `op` | int | nie | Operacja dziennika dla przebiegu. |
| `coverage` | object [`Coverage`](#coverage) | nie | Pokrycie kodu. |
| `baseline` | text | nie | Wcześniejszy przebieg, z którym porównano wyniki. |
| `regressions` | list<text> | nie | Testy, które zaliczono w bazie porównawczej, a teraz nie przechodzą. |
| `fixed` | list<text> | nie | Testy, które nie przechodziły w bazie porównawczej, a teraz przechodzą. |
| `slowest` | list<text> | nie | Najwolniejsze testy, każdy jako `id: <time>ms`. |

### test_result {#testresult}

Wynik jednego testu.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `run` | text | tak | Przebieg testów. |
| `id` | text | tak | Stabilny identyfikator, na przykład `tests/a_test.py::test_x`. |
| `name` | text | tak | Nazwa. |
| `status` | enum(passed\|failed\|skipped\|error\|todo) | tak | Wynik. |
| `suite` | text | nie | Zestaw. |
| `duration` | duration | nie | Czas, który upłynął. |
| `message` | text | nie | Komunikat o niepowodzeniu. |
| `path` | path | nie | Plik testu. |
| `line` | int | nie | Wiersz, od 1. |
| `column` | int | nie | Kolumna, od 1. |
| `stack` | text | nie | Ślad stosu. |
| `stdout` | text | nie | Standardowe wyjście testu. |
| `stderr` | text | nie | Standardowe wyjście błędów testu. |
| `retries` | int | nie | Liczba ponowień. |

### finding {#finding}

Ustalenie analizy lub przeglądu.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik. |
| `kind` | enum(types\|lint\|format\|complexity\|duplication\|conflict_markers\|whitespace\|large_files\|binary\|secrets\|subject\|other) | tak | Rodzaj. |
| `severity` | enum(error\|warning\|info) | tak | Waga. |
| `message` | text | tak | Komunikat. |
| `line` | int | nie | Wiersz, od 1. |
| `column` | int | nie | Kolumna, od 1. |
| `end_line` | int | nie | Ostatni wiersz. |
| `end_column` | int | nie | Kolumna za fragmentem w ostatnim wierszu. |
| `tool` | text | nie | Narzędzie, które je zgłosiło. |
| `id` | text | nie | Identyfikator, na przykład `an-3#17`. |
| `analysis` | text | nie | Analiza, na przykład `an-3`. |
| `rule` | text | nie | Reguła narzędzia, na przykład `no-unused-vars`. |
| `fixable` | bool | nie | Narzędzie może to naprawić. |
| `sample` | list<text> | nie | Wiersze fragmentu. |
| `value` | number | nie | Zmierzona wartość, na przykład złożoność. |
| `threshold` | number | nie | Przekroczony próg. |
| `related` | list<object [`RelatedLocation`](#relatedlocation)> | nie | Powiązane pozycje, na przykład druga kopia duplikatu. |
| `hunk` | text | nie | Identyfikator fragmentu przeglądu. |
| `target` | enum(file\|message) | nie | Czego dotyczy ustalenie; dla komunikatu commitu `path` jest korzeniem repozytorium. |
| `count` | int | nie | Liczba wystąpień zagregowanych w tym ustaleniu. |

### effect {#effect}

Skutek etapu zmieniającego lub uruchamiającego, jako rekord dla etapów następujących po skutku.

Pełny skutek opisuje raport skutku; ten rekord zawiera części, które późniejsze etapy filtrują i sprawdzają.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `status` | enum(ok\|planned\|running\|failed\|rolled_back) | tak | Status. |
| `kind` | text | tak | Polecenie, na przykład `replace`. |
| `op` | int | nie | Numer operacji dziennika. |
| `targets` | int | nie | Liczba celów. |
| `summary` | map<int> | nie | Liczniki podsumowania, na przykład `files_modified`. |
| `changes` | list<record [`change`](#change)> | nie | Zmienione pliki. |
| `notable` | list<text> | nie | Kody nietypowych skutków, na przykład `mass_delete`. |
| `irreversible` | list<object (wpis nieodwracalny raportu skutku)> | nie | Skutki, których nie można cofnąć. |
| `exec` | record [`exec`](#exec) | nie | Uruchomienie programu. |
| `job` | record [`job`](#job) | nie | Zadanie w tle. |
| `test` | record [`test_run`](#test_run) | nie | Przebieg testów. |
| `analysis` | record [`analysis`](#analysis) | nie | Analiza. |
| `target` | text | nie | Cel skutku poza systemem plików, na przykład `service:nginx`. |
| `action` | text | nie | Akcja na tym celu, na przykład `restart`. |
| `before` | any | nie | Stan przed. |
| `after` | any | nie | Stan po. |
| `git` | record [`git_effect`](#git_effect) | nie | Skutek dla repozytorium. |
| `return_value` | int | nie | Wartość zwrócona przez metodę wywołaną na instancji zarządzania systemu Windows. |

### git_effect {#giteffect}

Skutek operacji na repozytorium.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `repo` | path | tak | Korzeń repozytorium. |
| `state` | enum(completed\|conflicts\|stopped\|fast_forward\|up_to_date\|nothing_to_do) | tak | Wynik. |
| `head_before` | text | nie | Commit `HEAD` przed operacją. |
| `head_after` | text | nie | Commit `HEAD` po operacji. |
| `branch` | text | nie | Bieżąca gałąź po operacji. |
| `ref_updates` | list<record [`git_ref_update`](#git_ref_update)> | nie | Aktualizacje referencji. |
| `conflicts` | list<record [`git_conflict`](#git_conflict)> | nie | Konflikty pozostawione w drzewie roboczym. |
| `in_progress` | enum(none\|merge\|rebase\|cherry_pick\|revert\|bisect) | nie | Operacja pozostawiona w toku. |
| `step` | object [`GitStep`](#gitstep) | nie | Postęp operacji o wielu krokach. |
| `objects_written` | int | nie | Obiekty zapisane w bazie obiektów. |
| `objects_received` | int | nie | Obiekty otrzymane ze zdalnego repozytorium. |
| `bytes_received` | size | nie | Bajty otrzymane ze zdalnego repozytorium. |
| `bytes_sent` | size | nie | Bajty wysłane do zdalnego repozytorium. |
| `branch_before` | text | nie | Bieżąca gałąź przed operacją. |
| `detached` | bool | nie | `HEAD` wskazuje na commit, a nie na gałąź. |
| `upstream` | text | nie | Gałąź nadrzędna bieżącej gałęzi. |
| `ahead` | int | nie | Commity bieżącej gałęzi nieobecne w gałęzi nadrzędnej. |
| `behind` | int | nie | Commity gałęzi nadrzędnej nieobecne w bieżącej gałęzi. |
| `upstream_gone` | bool | nie | Skonfigurowana gałąź nadrzędna już nie istnieje. |
| `merge_head` | text | nie | Commit scalany w trakcie trwającego scalania. |
| `counts` | object [`GitCounts`](#gitcounts) | nie | Liczby zmienionych ścieżek według stanu. |
| `index` | object [`GitIndexSummary`](#gitindexsummary) | nie | Podsumowanie indeksu po dodaniu do poczekalni. |
| `checkpoint` | text | nie | Punkt kontrolny utworzony przez operację. |
| `checkpoint_before` | text | nie | Punkt kontrolny zapisany przed przywróceniem lub przełączeniem. |
| `state_token` | text | nie | Skrót stanu repozytorium dla parametru `expect_state` późniejszego wywołania. |

#### GitCounts {#gitcounts}

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `staged` | int | tak | Ścieżki ze zmianami w indeksie. |
| `unstaged` | int | tak | Ścieżki ze zmianami w drzewie roboczym nieobecnymi w indeksie. |
| `untracked` | int | tak | Ścieżki nieśledzone. |
| `conflicts` | int | tak | Ścieżki z konfliktami. |

#### GitIndexSummary {#gitindexsummary}

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `files` | int | tak | Pliki ze zmianami w poczekalni. |
| `added` | int | tak | Dodane wiersze. |
| `removed` | int | tak | Usunięte wiersze. |

### registry_value {#registryvalue}

Wartość w rejestrze systemu Windows.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Klucz, na przykład `registry:/HKCU/Software/Example`. |
| `name` | text | tak | Nazwa; pusta dla wartości domyślnej klucza. |
| `type` | enum(string\|expand_string\|multi_string\|dword\|qword\|binary\|none\|other) | tak | Typ. |
| `value` | any | tak | Wartość: tekst, int, lista tekstów lub bajty. |
| `size` | size | tak | Rozmiar w bajtach. |
| `view` | enum(64\|32) | nie | Widok rejestru. |

### journal_state {#journalstate}

Stan i limity dziennika operacji.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `active` | bool | tak | Dziennik jest aktywny. |
| `entries` | int | tak | Liczba wpisów. |
| `bytes` | size | tak | Przechowywane bajty. |
| `limits` | list<object [`JournalLimit`](#journallimit)> | tak | Aktywne limity wraz z ich źródłami. |
| `volume` | path | nie | Wolumin, gdy został wybrany. |
| `oldest` | instant | nie | Czas najstarszego wpisu. |
| `oldest_id` | int | nie | Numer najstarszego wpisu. |
| `newest_id` | int | nie | Numer najnowszego wpisu. |
| `pinned` | int | nie | Przypięte wpisy. |
| `reason` | text | nie | Dlaczego dziennik nie jest aktywny. |

### value {#value}

Wynik wyrażenia.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `value` | any | tak | Wartość. |
| `type` | text | tak | Typ wartości w języku. |

### group {#group}

Grupa rekordów o tym samym kluczu.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `key` | any | tak | Klucz: jedna wartość lub mapa dla wielu kluczy. |
| `count` | int | tak | Liczba rekordów. |
| `items` | list<any> | nie | Rekordy, gdy zażądano. |

### summary {#summary}

Statystyki pola.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `count` | int | tak | Liczba rekordów. |
| `field` | text | nie | Pole. |
| `group` | any | nie | Klucz grupy. |
| `sum` | any | nie | Suma. |
| `min` | any | nie | Najmniejsza wartość. |
| `max` | any | nie | Największa wartość. |
| `average` | any | nie | Średnia. |

### field_info {#fieldinfo}

Pole rekordów w potoku.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `field` | text | tak | Nazwa pola. |
| `type` | text | tak | Typ w języku. |
| `example` | any | nie | Przykładowa wartość. |
| `required` | bool | nie | Obecne w każdym rekordzie. |
| `records` | int | nie | Liczba rekordów, które je mają. |

### cim_object {#cimobject}

Obiekt zarządzania systemu Windows.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `class` | text | tak | Klasa. |
| `path` | text | tak | Ścieżka obiektu. |
| `properties` | map<any> | tak | Właściwości. |

### blame_line {#blameline}

Wiersz opatrzony informacją o commicie, który zmienił go jako ostatni.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | text | tak | Plik. |
| `no` | int | tak | Numer wiersza, od 1. |
| `commit` | text | tak | Identyfikator commitu. |
| `short_id` | text | tak | Skrócony identyfikator. |
| `author` | text | tak | Autor. |
| `time` | instant | tak | Czas autora. |
| `text` | text | tak | Tekst wiersza. |
| `author_email` | text | nie | Adres e-mail autora. |
| `original_path` | text | nie | Ścieżka w tym commicie. |
| `original_no` | int | nie | Numer wiersza w tym commicie. |
| `boundary` | bool | nie | Commit jest granicą historii. |

### hover {#hover}

Opis symbolu w danej pozycji.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik. |
| `line` | int | tak | Wiersz, od 1. |
| `column` | int | tak | Kolumna, od 1. |
| `source` | enum(lsp\|syntax) | tak | Źródło informacji. |
| `signature` | text | nie | Sygnatura. |
| `documentation` | text | nie | Dokumentacja. |

### code_action {#codeaction}

Akcja kodu oferowana przez serwer języka.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik. |
| `line` | int | tak | Wiersz, od 1. |
| `title` | text | tak | Tytuł. |
| `source` | enum(lsp\|syntax) | tak | Źródło informacji. |
| `end_line` | int | nie | Ostatni wiersz. |
| `kind` | text | nie | Rodzaj, na przykład `quickfix`. |

### language {#language}

Obsługiwany język i jego serwer.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `language` | text | tak | Język. |
| `extensions` | list<text> | tak | Rozszerzenia plików. |
| `server_found` | bool | tak | Znaleziono serwer. |
| `grammar_version` | text | nie | Wersja wbudowanej gramatyki. |
| `server` | text | nie | Program serwera. |
| `server_running` | bool | nie | Serwer działa. |
| `server_pid` | int | nie | Proces serwera. |
| `server_root` | path | nie | Katalog główny serwera. |

### rename_edit {#renameedit}

Planowana edycja zmiany nazwy.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik. |
| `line` | int | tak | Wiersz, od 1. |
| `column` | int | tak | Kolumna, od 1. |
| `end_line` | int | tak | Ostatni wiersz. |
| `end_column` | int | tak | Kolumna za fragmentem. |
| `old_text` | text | tak | Tekst przed zmianą. |
| `new_text` | text | tak | Tekst po zmianie. |
| `kind` | enum(code\|comment\|string\|import) | tak | Rodzaj tekstu. |
| `source` | enum(lsp\|syntax) | tak | Źródło informacji. |

### profile_entry {#profileentry}

Wpis profilu repozytorium.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `section` | enum(commands\|entry_points\|hotspots\|manifests) | tak | Sekcja. |
| `name` | text | tak | Nazwa. |
| `path` | path | nie | Plik. |
| `command` | text | nie | Polecenie. |
| `origin` | text | nie | Skąd pochodzi wpis, na przykład z manifestu. |
| `changes` | int | nie | Liczba zmian w historii. |
| `last_change` | instant | nie | Czas ostatniej zmiany. |

### ping_result {#pingresult}

Wynik testu osiągalności; czasy w milisekundach.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `host` | text | tak | Host w podanej postaci. |
| `address` | text | tak | Użyty adres. |
| `sent` | int | tak | Wysłane żądania. |
| `received` | int | tak | Otrzymane odpowiedzi. |
| `loss` | number | tak | Utrata w procentach. |
| `rtt_min` | number | nie | Najkrótszy czas obiegu. |
| `rtt_avg` | number | nie | Średni czas obiegu. |
| `rtt_max` | number | nie | Najdłuższy czas obiegu. |
| `rtt_stddev` | number | nie | Odchylenie standardowe czasu obiegu. |

### dns_record {#dnsrecord}

Rekord zasobu DNS.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Nazwa. |
| `type` | text | tak | Typ, na przykład `AAAA`. |
| `value` | text | tak | Dane. |
| `ttl` | duration | tak | Czas życia. |
| `server` | text | nie | Serwer, który odpowiedział. |

### network_interface {#networkinterface}

Interfejs sieciowy.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Nazwa. |
| `addresses` | list<text> | tak | Adresy z długością prefiksu. |
| `up` | bool | tak | Interfejs jest włączony. |
| `mac` | text | nie | Adres sprzętowy. |
| `mtu` | int | nie | Maksymalna jednostka transmisji. |
| `kind` | enum(ethernet\|wireless\|loopback\|virtual\|other) | nie | Rodzaj. |
| `gateway` | text | nie | Brama domyślna. |

### program_location {#programlocation}

Lokalizacja programu.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Nazwa. |
| `found` | bool | tak | Program został znaleziony. |
| `path` | path | nie | Ścieżka, na którą rozwiązuje się nazwa. |
| `version` | text | nie | Wersja. |
| `source` | enum(path\|package\|app\|catalog) | nie | Źródło. |
| `all_paths` | list<path> | nie | Każde dopasowanie w ścieżce wyszukiwania, w kolejności rozwiązywania. |
| `version_source` | enum(path\|package\|app\|catalog) | nie | Źródło, które podało wersję. |
| `alias_of` | text | nie | Program, którego aliasem jest nazwa. |

### program_category {#programcategory}

Kategoria programów.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Nazwa. |
| `count` | int | tak | Liczba programów. |
| `source` | text | tak | Źródło. |
| `title` | text | nie | Tytuł. |
| `description` | text | nie | Opis. |
| `subcategories` | list<text> | nie | Podkategorie. |
| `level` | enum(category\|subcategory) | nie | Poziom kategorii. |
| `parent` | text | nie | Kategoria, do której należy podkategoria. |
| `summary` | text | nie | Jedno zdanie. |
| `aliases` | list<text> | nie | Inne nazwy, które wskazują na kategorię. |
| `sources` | list<text> | nie | Źródła, w których zadeklarowano kategorię. |
| `counts_by_source` | map | nie | Liczba programów według źródła. |

### schedule_run {#schedulerun}

Jedno uruchomienie zaplanowanego zadania.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `schedule_id` | text | tak | Zaplanowane zadanie. |
| `started` | instant | tak | Czas rozpoczęcia. |
| `exit_state` | enum(ok\|failed\|timed_out\|skipped_overlap\|missed\|cancelled) | tak | Wynik. |
| `trigger_time` | instant | tak | Czas planowany. |
| `elapsed` | duration | nie | Czas, który upłynął. |
| `operation` | int | nie | Operacja dziennika dla uruchomienia. |
| `message` | text | nie | Komunikat. |

### analysis {#analysis}

Werdykt analizy.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `id` | text | tak | Identyfikator, na przykład `an-3`. |
| `passed` | bool | tak | Analiza zakończyła się powodzeniem. |
| `fail_on` | text | tak | Najniższa waga, która kończy analizę niepowodzeniem. |
| `errors` | int | tak | Znalezione błędy. |
| `warnings` | int | tak | Znalezione ostrzeżenia. |
| `infos` | int | tak | Znalezione informacje. |
| `checks` | list<object [`AnalysisCheck`](#analysischeck)> | tak | Wykonane kontrole. |
| `paths` | list<path> | tak | Analizowane ścieżki. |
| `changed_only` | bool | tak | Analizowano tylko zmienione pliki. |
| `started` | instant | tak | Czas rozpoczęcia. |
| `duration` | duration | tak | Czas, który upłynął. |
| `fingerprint` | text | nie | Odcisk stanu kodu. |
| `duplication_percent` | number | nie | Udział zduplikowanych wierszy w analizowanych plikach, w procentach. |
| `op` | int | nie | Operacja dziennika dla analizy. |

### analyze_tool {#analyzetool}

Narzędzie analizy znalezione w projekcie.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `tool` | text | tak | Narzędzie. |
| `language` | text | tak | Język. |
| `check` | enum(types\|lint\|format) | tak | Wykonywana kontrola. |
| `found` | bool | tak | Narzędzie zostało znalezione. |
| `version` | text | nie | Wersja. |
| `config` | path | nie | Plik konfiguracji. |
| `command` | list<text> | nie | Polecenie. |

### git_tag {#gittag}

Tag repozytorium.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Nazwa. |
| `target` | text | tak | Identyfikator oznaczonego obiektu. |
| `kind` | enum(lightweight\|annotated) | tak | Rodzaj. |
| `tagger` | text | nie | Autor tagu. |
| `time` | instant | nie | Czas. |
| `message` | text | nie | Komunikat. |
| `signature` | object [`CommitSignature`](#commitsignature) | nie | Podpis. |

### git_worktree {#gitworktree}

Drzewo robocze repozytorium.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Katalog. |
| `head` | text | tak | Identyfikator commitu `HEAD`. |
| `detached` | bool | tak | `HEAD` jest odłączony. |
| `bare` | bool | tak | Repozytorium typu bare. |
| `main` | bool | tak | Główne drzewo robocze. |
| `branch` | text | nie | Gałąź. |
| `locked` | text | nie | Przyczyna blokady. |
| `prunable` | bool | nie | Może zostać usunięte poleceniem prune. |

### git_stash {#gitstash}

Wpis schowka (stash).

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `index` | int | tak | Pozycja, 0 dla najnowszego. |
| `id` | text | tak | Identyfikator commitu. |
| `message` | text | tak | Komunikat. |
| `time` | instant | tak | Czas. |
| `includes_untracked` | bool | tak | Uwzględnione pliki nieśledzone. |
| `branch` | text | nie | Gałąź, na której utworzono wpis. |

### git_checkpoint {#gitcheckpoint}

Punkt kontrolny drzewa roboczego.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `id` | text | tak | Identyfikator, na przykład `cp-2`. |
| `commit` | text | tak | Identyfikator commitu. |
| `time` | instant | tak | Czas. |
| `ref` | text | tak | Referencja, na przykład `refs/nurt/checkpoints/cp-2`. |
| `index_included` | bool | tak | Indeks został uwzględniony. |
| `label` | text | nie | Etykieta. |
| `files` | int | nie | Liczba plików. |

### git_ref_update {#gitrefupdate}

Aktualizacja referencji przez transfer.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `ref` | text | tak | Referencja lokalna. |
| `status` | enum(new\|updated\|forced\|deleted\|up_to_date\|rejected\|error) | tak | Wynik. |
| `remote` | text | tak | Repozytorium zdalne. |
| `remote_ref` | text | nie | Referencja zdalna. |
| `old` | text | nie | Identyfikator przed zmianą. |
| `new` | text | nie | Identyfikator po zmianie. |
| `reason` | text | nie | Przyczyna odrzucenia, na przykład `non_fast_forward`. |

### git_conflict {#gitconflict}

Konflikt pozostawiony przez scalenie.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | text | tak | Ścieżka w repozytorium. |
| `raw_path` | bytes | nie | Bajty `path`; obecne tylko wtedy, gdy nie jest poprawnym Unicode. |
| `kind` | enum(both_modified\|both_added\|both_deleted\|added_by_us\|added_by_them\|deleted_by_us\|deleted_by_them\|rename_rename\|rename_delete\|directory_file\|binary\|submodule) | tak | Rodzaj. |
| `markers` | bool | tak | Plik zawiera znaczniki konfliktu. |
| `base` | text | nie | Identyfikator obiektu bazowego. |
| `ours` | text | nie | Identyfikator obiektu bieżącej strony. |
| `theirs` | text | nie | Identyfikator obiektu drugiej strony. |

### git_trash_entry {#gittrashentry}

Zachowana kopia usuniętej gałęzi lub odrzuconego wpisu schowka.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Nazwa usuniętej gałęzi lub wpisu schowka. |
| `kind` | enum(branch\|stash) | tak | Rodzaj. |
| `ref` | text | tak | Referencja przechowująca kopię, na przykład `refs/nurt/trash/feature`. |
| `commit` | text | tak | Identyfikator commitu. |
| `time` | instant | tak | Czas usunięcia. |
| `subject` | text | nie | Pierwszy wiersz komunikatu commitu. |

## Obiekty zagnieżdżone {#nested-objects}

### ContentImage {#contentimage}

Obraz przenoszony przez rekord content.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `mime` | text | tak | Typ nośnika, na przykład `image/png`. |
| `width` | int | nie | Szerokość w pikselach. |
| `height` | int | nie | Wysokość w pikselach. |
| `data` | bytes | nie | Dane obrazu. |

### MatchContainer {#matchcontainer}

Symbol obejmujący dopasowanie.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Nazwa symbolu. |
| `kind` | enum(function\|method\|class\|struct\|interface\|enum\|variable\|constant\|field\|module\|type\|macro\|other) | tak | Rodzaj symbolu. |
| `line` | int | tak | Pierwszy wiersz, od 1. |
| `end_line` | int | nie | Ostatni wiersz. |

### RelatedLocation {#relatedlocation}

Powiązana pozycja w pliku.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik. |
| `line` | int | tak | Wiersz, od 1. |
| `end_line` | int | nie | Ostatni wiersz. |
| `message` | text | nie | Wyjaśnienie. |

### OutputShown {#outputshown}

Jaka część jednego strumienia wyjścia jest pokazana.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `head` | int | tak | Wiersze pokazane od początku. |
| `tail` | int | tak | Wiersze pokazane od końca. |
| `omitted_lines` | int | tak | Wiersze pominięte między nimi. |
| `filtered` | int | tak | Wiersze usunięte przez filtry wyjścia. |

### ExecShown {#execshown}

Jaka część wyjścia uruchomienia jest pokazana.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `stdout` | object [`OutputShown`](#outputshown) | nie | Standardowe wyjście. |
| `stderr` | object [`OutputShown`](#outputshown) | nie | Standardowe wyjście błędów. |

### ExecDiagnostic {#execdiagnostic}

Błąd znaleziony w wyjściu znanego narzędzia; każde pole jest jednym wierszem.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `file` | text | nie | Plik wskazany przez narzędzie. |
| `line` | int | nie | Wiersz, od 1. |
| `col` | int | nie | Kolumna, od 1. |
| `code` | text | nie | Kod błędu, na przykład `E0308` lub `TS2322`. |
| `message` | text | tak | Komunikat. |
| `severity` | enum(error\|warning\|info\|hint) | tak | Waga. |
| `count` | int | nie | Miejsca, w których zgłoszono ten sam błąd, gdy jest ich więcej niż jedno. |

### ExecDiagnostics {#execdiagnostics}

Błędy i ostrzeżenia znalezione w wyjściu znanego narzędzia.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `tool` | text | tak | Narzędzie, którego format rozpoznano: `cargo`, `pytest`, `tsc`, `gcc` (gcc lub clang) albo `go`. |
| `errors` | int | tak | Liczba błędów w wyjściu. |
| `warnings` | int | tak | Liczba ostrzeżeń w wyjściu. |
| `first` | list<object [`ExecDiagnostic`](#execdiagnostic)> | tak | Pierwsze różne błędy, co najwyżej 10. |

### Progress {#progress}

Postęp uruchomienia.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `percent` | number | nie | Wartość procentowa, gdy wyjście ją pokazuje. |
| `line` | text | tak | Ostatni wiersz postępu. |

### Batch {#batch}

Jedna partia uruchomienia na strumieniu rekordów.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `no` | int | tak | Numer partii, od 1. |
| `args` | list<text> | tak | Argumenty partii. |
| `exit_code` | int | tak | Kod wyjścia. |
| `elapsed` | duration | tak | Czas, który upłynął. |

### CoverageFile {#coveragefile}

Pokrycie jednego pliku.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | path | tak | Plik. |
| `lines_pct` | number | tak | Procent pokrytych wierszy. |
| `uncovered` | list<range> | nie | Zakresy niepokrytych wierszy. |

### Coverage {#coverage}

Pokrycie kodu przebiegu testów.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `lines_pct` | number | tak | Procent pokrytych wierszy. |
| `branches_pct` | number | nie | Procent pokrytych gałęzi. |
| `functions_pct` | number | nie | Procent pokrytych funkcji. |
| `statements_pct` | number | nie | Procent pokrytych instrukcji. |
| `files` | list<object [`CoverageFile`](#coveragefile)> | nie | Pokrycie według pliku. |
| `report_file` | path | nie | Plik raportu zapisany przez framework. |
| `format` | enum(lcov\|cobertura\|istanbul\|go\|llvm) | nie | Format pliku raportu. |

### AnalysisCheck {#analysischeck}

Jedna kontrola analizy.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `check` | text | tak | Kontrola, na przykład `lint`. |
| `tool` | text | nie | Narzędzie, które ją wykonało. |
| `status` | enum(ok\|failed\|skipped\|missing) | tak | Wynik. |
| `findings` | int | nie | Liczba ustaleń. |
| `duration` | duration | nie | Czas, który upłynął. |

### Trigger {#trigger}

Wyzwalacz zaplanowanego zadania.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `kind` | enum(cron\|every\|at) | tak | Rodzaj wyzwalacza. |
| `expression` | text | tak | Wyrażenie cron, interwał lub punkt w czasie w zapisanej postaci. |
| `timezone` | text | nie | Strefa czasowa wyzwalacza. |

### CommitFile {#commitfile}

Plik zmieniony przez commit.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `path` | text | tak | Ścieżka w repozytorium. |
| `status` | enum(unmodified\|modified\|added\|deleted\|renamed\|copied\|type_changed\|unmerged\|untracked\|ignored) | tak | Zmiana. |
| `old_path` | text | nie | Ścieżka przed zmianą nazwy. |
| `added` | int | nie | Dodane wiersze. |
| `removed` | int | nie | Usunięte wiersze. |
| `binary` | bool | nie | Treść binarna. |

### CommitSignature {#commitsignature}

Podpis commitu lub tagu.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `status` | enum(none\|good\|bad\|unknown_key\|unsupported) | tak | Stan weryfikacji. |
| `format` | enum(openpgp\|ssh\|x509) | nie | Format. |
| `key` | text | nie | Identyfikator lub odcisk klucza. |
| `signer` | text | nie | Podpisujący. |

### CommitStats {#commitstats}

Statystyki zmian commitu.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `files` | int | tak | Zmienione pliki. |
| `added` | int | tak | Dodane wiersze. |
| `removed` | int | tak | Usunięte wiersze. |

### Submodule {#submodule}

Stan podmodułu.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `commit_changed` | bool | tak | Zapisany commit uległ zmianie. |
| `modified` | bool | tak | Śledzona treść została zmodyfikowana. |
| `untracked` | bool | tak | Obecna jest treść nieśledzona. |

### JournalLimit {#journallimit}

Aktywny limit dziennika.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `name` | text | tak | Limit: `steps`, `age` lub `size`. |
| `value` | any | tak | Wartość, na przykład int, duration lub size; `unlimited` jako wartość wyliczenia. |
| `source` | text | tak | Skąd pochodzi wartość: `default`, `system`, `user`, `project`, `session` lub `floor`. |
| `scope` | text | nie | Zakres wartości ustawionej w trakcie pracy: `session` lub `user`. |
| `floor` | any | nie | Dolna granica ochrony, poniżej której wartość nie może spaść. |
| `expires` | int | nie | Wpisy, które wygasają z powodu obniżonego limitu. |

### GitStep {#gitstep}

Postęp operacji o wielu krokach, takiej jak rebase.

| Pole | Typ | Wymagane | Znaczenie |
|---|---|---|---|
| `done` | int | tak | Wykonane kroki. |
| `total` | int | tak | Kroki łącznie. |

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
