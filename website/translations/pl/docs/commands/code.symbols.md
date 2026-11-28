---
command: code.symbols
group: code
summary: Znajduje definicje symboli według nazwy w całym repozytorium.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.symbols

## Składnia {#synopsis}

```text
code.symbols [<name>] [name: value]...
```

## Opis {#description}

Wyświetla definicje, których nazwy pasują do `name`, w podanych plikach
i katalogach, jeden rekord `symbol` na definicję.

Przy `source: auto` i podanej nazwie najpierw odpowiada serwer języka projektu.
Gdy żaden serwer nie odpowiada, gdy nie podano nazwy oraz przy
`exported_only`, symbole pochodzą z analizy składniowej z wbudowanymi
gramatykami, a awaria serwera jest zgłaszana jako `semantic_fallback`.

Analiza składniowa katalogu korzysta z indeksu symboli przechowywanego
w katalogu stanu. Późniejsze wyszukiwanie odczytuje tylko pliki, których
rozmiar lub czas modyfikacji uległ zmianie. Wyliczanie stosuje pliki ignorowania
i pomija katalogi kontroli wersji, zależności oraz pamięci podręcznej. Katalogi
`.git`, `node_modules`, `__pycache__`, `venv` i `.venv` są zawsze pomijane, tak
samo jak pliki większe niż `code.file_bytes_max`. Indeks zawiera nie więcej niż
`code.index_max_files` plików. Po osiągnięciu limitu wynik jest niepełny,
a raport zawiera `source_unavailable`.

Wyszukiwanie w katalogu obejmuje języki programowania. Formaty danych i znaczników
(JSON, YAML, TOML, Markdown, HTML, CSS, XML) są przeszukiwane, gdy nazywa je
`language` albo gdy `path` wskazuje plik. Pliki bez rozszerzenia są
przeszukiwane, gdy ich wiersz shebang lub modeline wskazuje język.

Nazwy są oceniane od 6 (ten sam tekst) do 1 (litery zapytania występują
w kolejności). Rekordy są uporządkowane według oceny, a następnie według ścieżki
i położenia. Bez nazwy każdy symbol jest wyświetlany w kolejności ścieżek
i źródła.

| Ocena | Warunek | Tryby |
|---|---|---|
| 6 | ten sam tekst | wszystkie |
| 5 | ten sam tekst bez względu na wielkość liter; dla `regex` równy wzorcowi | wszystkie |
| 4 | zaczyna się od zapytania, bez względu na wielkość liter | `prefix`, `substring`, `fuzzy` |
| 3 | zawiera zapytanie, bez względu na wielkość liter | `substring`, `fuzzy` |
| 2 | słowa zapytania zaczynają kolejne słowa nazwy; każde dopasowanie `regex` | `fuzzy`, `regex` |
| 1 | litery i cyfry zapytania występują w kolejności | `fuzzy` |

Słowa są dzielone na granicach camelCase i na znakach innych niż litery i cyfry,
więc `getUserById`, `get_user_by_id` i `GetUserByID` mają wspólne słowa `get`,
`user`, `by`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text \| regex \| list<text>` | brak | Nazwa symbolu, wzorzec lub nazwy; po pominięciu wszystkie symbole. |
| `path` | `list<path>` | brak | Pliki lub katalogi do przeszukania. |
| `glob` | `glob` | `"**"` | Tylko pliki, których ścieżka pasuje do tego wzorca. |
| `kind` | `enum(function\|method\|class\|struct\|interface\|enum\|variable\|constant\|field\|module\|type\|macro\|any)` | `any` | Rodzaj symbolu. |
| `match` | `enum(exact\|prefix\|substring\|fuzzy\|regex)` | `fuzzy` | Sposób dopasowania nazwy. |
| `language` | `text` | `"auto"` | Język programowania; auto wykrywa go na podstawie rozszerzeń plików. |
| `source` | `enum(auto\|lsp\|syntax)` | `auto` | Serwer języka, analiza składniowa albo serwer, gdy jest dostępny. |
| `with_signature` | `bool` | `false` | Dołącza sygnaturę każdego symbolu. |
| `with_references` | `bool` | `false` | Dołącza liczbę odwołań do każdego symbolu. |
| `exported_only` | `bool` | `false` | Tylko symbole widoczne poza swoim modułem. |
| `exclude` | `list<glob>` | `[]` | Ścieżki do pominięcia, w składni gitignore. |
| `exclude_preset` | `list<enum(vcs\|deps\|build\|cache\|tests)>` | `[]` | Predefiniowane grupy ścieżek do pominięcia, takie jak zależności lub pliki testowe. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `symbol` |

Reguły widoczności: Python, nazwa niezaczynająca się od `_`; Go, nazwa pisana
wielką literą; Rust, modyfikator widoczności; JavaScript i TypeScript,
`export` na najwyższym poziomie; C i C++, brak `static`; Java i C#, `public`;
Kotlin, brak `private`, `protected` lub `internal`; Swift, `public` lub `open`;
PHP, brak `private` lub `protected`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |
| External use | `language_server`; Używa serwera języka, gdy jest zainstalowany, a w przeciwnym razie analizy składniowej; parametr source wybiera analizę jawnie. |

## Przykłady {#examples}

Znalezienie funkcji po skrócie jej słów.

```nurt example
code.symbols "getUser" kind: function
```

Wyświetlenie eksportowanych klas katalogu bez plików testowych.

```nurt example
code.symbols path: ["/home/user/app/src"] kind: class exported_only: true exclude_preset: [tests]
```

Znalezienie handlerów wraz z sygnaturami i liczbami odwołań z analizy składniowej.

```nurt example
code.symbols /Handler$/ match: regex source: syntax with_signature: true with_references: true
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | Nieznana wartość `match`, `kind` lub `exclude_preset` albo niepoprawny glob. | Użyj wartości wymienionej w sekcji Parametry. |
| `E_REGEX` | Wyrażenie regularne w `name` jest niepoprawne. | Popraw wzorzec. |
| `E_UNSUPPORTED_LANGUAGE` | `language` nie nazywa żadnego znanego języka albo język pliku podanego w `path` jest nieznany. | Sprawdź nazwy za pomocą `code.languages`. |
| `E_PARAM_CONFLICT` | `exported_only` z `source: lsp`: serwery nie zgłaszają widoczności. | Użyj `source: syntax` lub `auto`. |
| `E_EXTERNAL_MISSING` | `source: lsp` bez dostępnego serwera języka. | Zainstaluj serwer wskazany w `fix`. |
| `E_MISSING_PARAM` | Brak `path` i brak katalogu bazowego. | Podaj `path`. |
| `E_LIMIT` | Plik podany w `path` jest większy niż `code.file_bytes_max`. | Zwiększ limit albo podaj katalog. |
| `E_NOT_FOUND` | Ścieżka nie istnieje. | Sprawdź ścieżkę. |

## Powiązane {#related}

`code.outline`, `code.definition`, `code.references`, `code.languages`,
klucze konfiguracji `code.index_max_files`, `code.file_bytes_max`.
