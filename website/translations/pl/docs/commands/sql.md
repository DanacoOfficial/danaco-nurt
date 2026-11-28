---
command: sql
group: documents
summary: Wykonuje instrukcje SQL na pliku bazy danych SQLite; instrukcja, która cokolwiek zmienia, czyni z wywołania zmianę.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# sql

## Składnia {#synopsis}

```text
sql <path> <query> [name: value]...
```

## Opis {#description}

`sql` wykonuje instrukcje SQL z `query` na pliku bazy danych SQLite `path` i
daje jeden rekord `row` na każdy wiersz wyniku. Instrukcje są wykonywane po
kolei; instrukcje bez kolumn wyniku nie dają rekordów.

Wywołanie, którego instrukcje tylko odczytują (`SELECT`, `VALUES`, `EXPLAIN`,
`WITH` bez `INSERT`, `UPDATE` lub `DELETE`, pragma, która tylko odczytuje),
otwiera bazę danych tylko do odczytu i ustawia połączenie na `query_only`.
Wywołanie z instrukcją, która cokolwiek zmienia, otwiera bazę danych do zapisu
i jest skutkiem; żadna odmowa nie żąda `write: true`. Instrukcjami, które
czynią z wywołania zmianę, są każda inna instrukcja, pragma, która ustawia
wartość lub działa, oraz wywołanie `load_extension`. Zapytanie, które nie jest
literalnym tekstem, jest zmianą. `write: true` pozostaje opcjonalną formą
jawną. `write: false` utrzymuje wywołanie tylko do odczytu: instrukcja, która
zmienia bazę danych, kończy się wtedy błędem `E_SQL` (`problem`:
`write_required`), a błąd zawiera to samo wywołanie bez `write: false` jako
`fix`. Każda instrukcja odczytu jest przygotowywana i sprawdzana przed
wytworzeniem pierwszego wiersza.

Zmiana zapisuje w dzienniku plik bazy danych oraz jego pliki `-wal`, `-shm` i
`-journal` przed wykonaniem pierwszej instrukcji. Instrukcje są wykonywane w
jednej transakcji `BEGIN IMMEDIATE`. Transakcja jest zatwierdzana, gdy każda
instrukcja się powiedzie, i wycofywana, gdy któraś zawiedzie; nic się wtedy
nie zmienia. Po zatwierdzeniu dziennik zapisu z wyprzedzeniem każdej bazy
danych jest zapisywany do jej pliku punktem kontrolnym `TRUNCATE`, więc pliki
zawierają nowy stan.

| Instrukcja | Reguła |
|---|---|
| `BEGIN`, `COMMIT`, `END` | scalane z transakcją wywołania; niczego nie zmieniają |
| `ROLLBACK` | wycofuje wywołanie i kończy je błędem `E_ROLLED_BACK`; baza danych i dziennik pozostają takie jak wcześniej |
| `SAVEPOINT`, `RELEASE`, `ROLLBACK TO` | wykonywane wewnątrz transakcji |
| `ATTACH` | dozwolone; wykonywane poza transakcją (transakcja jest zatwierdzana przed nim i otwierana ponownie dla następnej zmiany). Plik jest obliczany (literał, parametr lub dowolne wyrażenie, wcześniej obliczone za pomocą `SELECT`), rozwiązywany jak każdy argument ścieżki (plik względny wymaga `base` lub `scope.base`, w przeciwnym razie `E_PATH_RELATIVE`) i zapisywany w dzienniku przed instrukcją; URI `file:` jest odczytywany do swojej ścieżki, a `mode=memory` lub `:memory:` nie wymaga pliku. SQLite otrzymuje wtedy rozwiązaną nazwę, więc otwiera plik, który dziennik zachował. Nowy plik jest zapisywany w dzienniku jako utworzony, a `undo` go usuwa. Gdy pliku nie można ustalić (URI z hostem, wyrażenie, które nie daje tekstu), wywołanie jest oznaczane jako nieodwracalne (`external_effect`) przed zmianą |
| `DETACH` | dozwolone; wykonywane na swoim miejscu, po zatwierdzeniu transakcji |
| `VACUUM`, `VACUUM schema` | wykonywane automatycznie poza transakcją, po pozostałych instrukcjach; plik bazy danych jest najpierw zapisywany w dzienniku |
| `VACUUM INTO` | dozwolone; wykonywane na swoim miejscu po zatwierdzeniu wcześniejszych instrukcji, więc kopia zawiera ich wynik. Nowy plik jest rozwiązywany i zapisywany w dzienniku jako utworzony tak jak plik `ATTACH`, więc `undo` go usuwa |
| pragmy, które ustawiają wartość lub działają | wykonywane poza transakcją, na swoim miejscu, więc `journal_mode`, `foreign_keys` i `temp_store_directory` działają; `temp_store_directory` i `data_store_directory` są dozwolone i zmieniają stan całego procesu Nurt (SQLite przechowuje je w zmiennych globalnych); wywołanie zgłasza każdą z nich jako skutek, który nie jest cofany, a `undo` ich nie resetuje |
| podgląd | plan wylicza skutki `load_extension` i dwóch pragm jako wpisy nieodwracalne (`external_effect`, „z tekstu instrukcji”); uruchomienie zgłasza to, co zobaczył autoryzator |
| `load_extension` | dozwolone w dowolnym zapisie (w cudzysłowie lub inną wielkością liter); raport oznacza wywołanie jako `external_effect`, ponieważ skutki rozszerzenia są nieznane i nie można ich cofnąć |

Wywołanie, które zmienia bazę danych, wyłącza tryb defensywny SQLite, ponieważ
język dopuszcza `load_extension`; schemat pozostaje niezaufany. Wywołanie z
`write: false` utrzymuje tryb defensywny i odmawia `ATTACH`, `DETACH`, pragm
katalogów (`problem`: `attach`, `pragma`) oraz `load_extension` (`problem`:
`extension`).

`path` musi wskazywać istniejący plik. Pusty plik jest pustą bazą danych, tak
jak w SQLite. Plik, który nie zaczyna się od nagłówka SQLite, taki jak
zaszyfrowana baza danych, kończy się błędem `E_DOC_PARSE`.

Wartości parametrów wiążą się z symbolami zastępczymi instrukcji. `params` jest
listą albo mapą.

Lista wiąże w kolejności instrukcji: każda instrukcja bierze tyle wartości, ile
ma parametrów. Symbole pozycyjne (`?`), numerowane (`?2`) i nazwane (`:id`,
`@id`, `$id`) biorą swoją wartość według indeksu parametru; symbol nazwany ma
indeks swojego pierwszego wystąpienia. Liczba wartości musi być równa liczbie
parametrów wszystkich instrukcji.

Mapa wiąże symbole nazwane według nazwy. Klucz jest zapisywany z przedrostkiem
symbolu lub bez niego, więc `id` i `:id` wiążą oba `:id`. Każdy symbol nazwany
wymaga wpisu, a każdy wpis musi być użyty przez symbol. Symbolu pozycyjnego
(`?`, `?2`) nie można związać z mapy. Naruszenie kończy się błędem `E_SQL` i
problemem `parameter_name`.

| Typ wartości | Wartość SQLite |
|---|---|
| `null` | `NULL` |
| `bool` | `INTEGER` 0 lub 1 |
| `int`, `size` (bajty), `duration` (milisekundy) | `INTEGER` |
| `number` | `REAL` |
| `text`, `path`, `glob`, `key`, `enum`, `decimal`, `date` | `TEXT` |
| `instant` | `TEXT` w postaci ISO 8601 |
| `bytes` | `BLOB` |

| Wartość SQLite | Typ komórki |
|---|---|
| `INTEGER` | `int` |
| `REAL` | `number`; nieskończoność jest tekstem `Inf` lub `-Inf` |
| `TEXT` | `text`; tekst, który nie jest UTF-8, jest `bytes` |
| `BLOB` | `bytes` |
| `NULL` | `null` |

Odczyt bazy danych w trybie dziennika zapisu z wyprzedzeniem tworzy pliki
`-wal` i `-shm`, gdy ich brakuje, a katalog jest zapisywalny, jak robi to
każdy czytnik SQLite; sam plik bazy danych nie jest zmieniany.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik bazy danych SQLite. |
| `query` | `text` | wymagany | Instrukcje SQL. |
| `params` | `list<any> \| map<any>` | `[]` | Wartości wiązane z symbolami zastępczymi instrukcji: lista dla symboli pozycyjnych, mapa z nazw na wartości dla symboli nazwanych. |
| `write` | `bool` | brak | Opcjonalny: instrukcja, która zmienia bazę danych, otwiera ją do zapisu także bez niego. write: false utrzymuje wywołanie tylko do odczytu. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `none` | `row` |

Pola `row`:

Wiersze są wytwarzane w trakcie wykonywania zapytania. Potok, który używa
tylko pierwszych wierszy, albo anulowane wykonanie przerywa wykonywaną
instrukcję.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `builtin` |
| Warunki odwracalności | Wywołanie, które zmienia bazę danych, najpierw kopiuje plik bazy danych i jego pliki dziennika, a cofnięcie je przywraca; pliki ATTACH i VACUUM INTO są zachowywane lub usuwane w ten sam sposób. Bez miejsca na kopię oraz dla skutków load_extension zmiana jest nieodwracalna. |

| Akcja | Skutek | Odwracalność | Uwaga |
|---|---|---|---|
| `write` | `local` | `reversible_if_preserved` | Dotyczy przypadku, gdy write ma wartość true, oraz bez write, gdy instrukcja cokolwiek zmienia. |

Przy zmianie dziennik operacji zachowuje plik bazy danych i znajdujące się obok
pliki `-wal`, `-shm` i `-journal` przed wykonaniem pierwszej instrukcji.
`undo` przywraca bazę danych i te pliki do ich poprzednich bajtów oraz usuwa
pliki dziennika utworzone przez zmianę. Bez miejsca na kopię zmiana jest
wykonywana i zgłaszana jako nieodwracalna z `no_space_for_copy`. Rekord zmiany
pliku bazy danych zgłasza liczbę wstawionych, zaktualizowanych lub usuniętych
wierszy, łącznie z wyzwalaczami, w `rows_changed`.

Blokada utrzymywana przez inne połączenie jest oczekiwana przez
`sql.busy_timeout` (domyślnie 5 sekund), a następnie kończy się błędem
`E_IN_USE`.

## Przykłady {#examples}

Utworzenie tabeli w pustym pliku bazy danych i zwrócenie wstawionego wiersza:

```nurt example
sql "/home/user/app.db" "create table users(id integer primary key, name text); insert into users(name) values (?) returning id, name" params: ["ada"]
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app.db", "type": "file", "content_utf8": "" }
] }
```

Wylistowanie tabel bazy danych:

```nurt example
sql "/home/user/app.db" "select name from sqlite_master where type = 'table' order by name"
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app.db", "type": "file", "content_utf8": "" }
] }
```

Powiązanie parametrów nazwanych z mapy:

```nurt example
sql "/home/user/app.db" "select :low as low, :high as high" params: {"low": 1, "high": 9}
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app.db", "type": "file", "content_utf8": "" }
] }
```

Obliczenie wartości z powiązanych parametrów za pomocą symboli numerowanych:

```nurt example
sql "/home/user/app.db" "select ?2 - ?1 as span, upper(?3) as label" params: [5, 12, "week"]
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app.db", "type": "file", "content_utf8": "" }
] }
```

Odpowiedź, która pomija rekordy, kończy się wskazówką "more: next" i zawiera
pole `continuation`; polecenie `next` zwraca następną część bez ponownego
uruchamiania polecenia.

```text
matched 5000  shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_SQL` | instrukcji nie można przygotować ani wykonać, zmienia bazę danych przy `write: false`, jest odrzucona przy `write: false` (`problem`: `attach`, `extension`, `pragma`), nie zawiera instrukcji (`problem`: `empty_query`) albo `params` nie pasuje do instrukcji (`problem`: `parameter_count`, `parameter_name`, `parameter_type`, `parameter_range`); problem `write_required` oznacza instrukcję, która zmienia bazę danych przy `write: false`, łącznie z `VACUUM INTO`; `details` zawiera `statement` i `sqlite_code` | popraw instrukcję, usuń `write: false` albo przekaż jedną wartość na parametr lub jeden wpis na symbol nazwany |
| `E_ROLLED_BACK` | instrukcja `ROLLBACK` zakończyła wywołanie; `details.statement` zawiera jej numer; komunikat i `details.irreversible` wskazują skutki, które zgłoszono wcześniej i których nie można cofnąć | usuń instrukcję `ROLLBACK`, aby zachować zmiany |
| `E_MISSING_PARAM` | nie podano `path` lub `query` | przekaż oba |
| `E_TYPE` | `path`, `query`, `params` lub `write` ma inny typ | przekaż `params` jako listę lub mapę |
| `E_DOC_PARSE` | plik nie jest bazą danych SQLite, jest zaszyfrowany lub uszkodzony; `details.format` ma wartość `sqlite`, a `details.sqlite_code` zawiera kod SQLite, gdy SQLite go zgłosił | przekaż plik bazy danych SQLite |
| `E_NOT_FOUND` | plik bazy danych nie istnieje | sprawdź ścieżkę; pusty plik jest pustą bazą danych |
| `E_PATH_RELATIVE` | plik `ATTACH` lub `VACUUM INTO` jest względny, a wywołanie nie ma `base` | podaj plik bezwzględny lub ustaw `base` |
| `E_IN_USE` | inne połączenie utrzymuje blokadę dłużej niż `sql.busy_timeout` | ponów później lub zwiększ `sql.busy_timeout` |
| `E_IO` | pliku nie można odczytać ani zapisać z innego powodu | zobacz komunikat |
| `E_PERMISSION` | pliku lub jego katalogu nie można odczytać ani zapisać | sprawdź uprawnienia |
| `E_NO_SPACE` | wolumin jest pełny podczas zmiany | zwolnij miejsce i uruchom wywołanie ponownie |
| `E_LIMIT` | instrukcje zmiany zwracają więcej wierszy, niż może pomieścić jedno wykonanie | zwracaj mniej wierszy z `RETURNING` lub `SELECT` |
| `E_CANCELLED` | wykonanie zostało anulowane albo upłynął jego limit czasu; zmiana jest wycofywana | uruchom wywołanie ponownie |
| `E_TIMEOUT` | wykonanie zostało anulowane albo upłynął jego limit czasu; zmiana jest wycofywana | uruchom wywołanie ponownie |

## Powiązane {#related}

`query` dla JSON i innych dokumentów strukturalnych, `set` dla wartości pod
ścieżkami kluczy, `undo` do cofnięcia zmiany.
