---
command: code.implementations
group: code
summary: Znajduje implementacje interfejsu lub typy pochodne od symbolu.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.implementations

## Składnia {#synopsis}

```text
code.implementations <path> [name: value]...
```

## Opis {#description}

Znajduje implementacje interfejsu, cechy (trait) lub metody abstrakcyjnej
w danym położeniu pliku `path` albo definicji wskazanej przez `symbol` oraz
typy pochodne od klasy.

Najpierw odpowiada serwer języka. Przy `source: auto` bez użytecznego serwera
albo przy pustej odpowiedzi analiza składniowa wyświetla klasy, struktury,
interfejsy i wyliczenia projektu, których deklaracja wymienia identyfikator po
`extends`, `implements` lub `for`, po `impl` albo na liście typów bazowych.
Wynik zawiera wtedy notatkę `semantic_fallback` z przyczyną. Analiza
składniowa nie śledzi aliasów ani importów pod inną nazwą.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik źródłowy. |
| `line` | `int` | brak | Wiersz symbolu, liczony od 1. |
| `column` | `int` | brak | Kolumna symbolu, liczona od 1. |
| `symbol` | `text` | brak | Nazwa symbolu, opcjonalnie kwalifikowana jako Container.name, zamiast położenia. |
| `language` | `text` | `"auto"` | Język programowania; auto wykrywa go na podstawie rozszerzenia pliku. |
| `root` | `path` | wykrywany na podstawie położenia pliku | Katalog główny projektu dla serwera języka. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `symbol` |

Pola rekordu `symbol` opisano w `code.definition`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `language_server` |
| Rozwiązanie zapasowe | `builtin` |

## Przykłady {#examples}

Wyświetlenie klas, które implementują interfejs.

```nurt example
code.implementations "/srv/app/src/repo.ts" symbol: "Repository"
```

Wyświetlenie implementacji metody cechy w danym położeniu.

```nurt example
code.implementations "/srv/app/src/store.rs" line: 18 column: 8
```

Wyświetlenie klas pochodnych wraz z ich plikami.

```nurt example
code.implementations "/srv/app/src/shapes.py" symbol: "Shape" |> select name path line
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Nie podano ani `line`, ani `symbol`. | Podaj `line` lub `symbol`. |
| `E_INVALID_VALUE` | Plik ma mniej wierszy niż `line` albo `source` jest niepoprawne. | Sprawdź parametry. |
| `E_SYMBOL_NOT_FOUND` | `symbol` nie ma definicji w pliku. | Sprawdź nazwę za pomocą `code.symbols`. |
| `E_SYMBOL_AMBIGUOUS` | `symbol` ma w pliku kilka definicji. | Zakwalifikuj nazwę jako `Container.name`. |
| `E_UNSUPPORTED_LANGUAGE` | Potrzebna jest analiza składniowa, a dla pliku nie ma dostępnej gramatyki. | Podaj `language`. |
| `E_EXTERNAL_MISSING` | Przy `source: lsp` dla języka nie ma dostępnego serwera. | Zainstaluj serwer wskazany w `fix`. |
| `E_EXTERNAL_FAILED` | Przy `source: lsp` serwer zakończył się awarią. | Zobacz komunikat serwera. |
| `E_TIMEOUT` | Przy `source: lsp` serwer nie odpowiedział w czasie `code.timeout`. | Zwiększ `code.timeout` albo użyj `source: auto`. |

## Powiązane {#related}

`code.definition`, `code.type_definition`, `code.symbols`.
