---
command: code.definition
group: code
summary: Znajduje definicję symbolu w danym położeniu lub o danej nazwie.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.definition

## Składnia {#synopsis}

```text
code.definition <path> [name: value]...
```

## Opis {#description}

Znajduje definicję identyfikatora w wierszu `line` i kolumnie `column` pliku
`path` albo definicję wskazaną przez `symbol` w `path`. Parametr `symbol` może
być kwalifikowany przez swój kontener (`Circle.area`); nazwa o kilku
definicjach w pliku kończy się błędem `E_SYMBOL_AMBIGUOUS` i wymienia
kandydatów. Bez `column` używany jest pierwszy identyfikator wiersza.

Najpierw odpowiada serwer języka danego pliku. Przy `source: auto` brakujący,
niedziałający lub wolny serwer albo pusta odpowiedź ustępują analizie
składniowej: definicjom identyfikatora w pliku, a następnie w plikach projektu
w tym samym języku. Wynik zawiera wtedy notatkę `semantic_fallback`
z przyczyną. Anulowane żądanie nigdy nie jest zastępowane. Przy `source: lsp`
odpowiedź serwera jest ostateczna, także gdy jest pusta.

Katalogiem głównym projektu jest najbliższy katalog nad plikiem, który zawiera
znacznik projektu (`pyproject.toml`, `package.json`, `tsconfig.json`, `go.mod`,
`Cargo.toml`, `compile_commands.json`, `.git`), albo katalog pliku.

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

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `language_server` |
| Rozwiązanie zapasowe | `builtin` |

## Przykłady {#examples}

Znalezienie definicji identyfikatora w wierszu 12, kolumnie 8.

```nurt example
code.definition "/srv/app/src/main.py" line: 12 column: 8
```

Znalezienie metody po jej nazwie kwalifikowanej.

```nurt example
code.definition "/srv/app/src/shapes.py" symbol: "Circle.area"
```

Znalezienie definicji symbolu w kolumnie danego wiersza.

```nurt example
code.definition "/srv/app/src/index.ts" line: 40 column: 15
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Nie podano ani `line`, ani `symbol`. | Podaj `line` lub `symbol`. |
| `E_INVALID_VALUE` | Plik ma mniej wierszy niż `line` albo `source` lub `root` jest niepoprawne. | Sprawdź położenie i parametry. |
| `E_SYMBOL_NOT_FOUND` | `symbol` nie ma definicji w pliku. | Sprawdź nazwę za pomocą `code.symbols` albo podaj `line` i `column`. |
| `E_SYMBOL_AMBIGUOUS` | `symbol` ma w pliku kilka definicji; próbki je wymieniają. | Zakwalifikuj nazwę jako `Container.name` albo podaj `line` i `column`. |
| `E_UNSUPPORTED_LANGUAGE` | Potrzebna jest analiza składniowa, a dla pliku nie ma dostępnej gramatyki. | Podaj `language`. |
| `E_EXTERNAL_MISSING` | Przy `source: lsp` dla języka nie ma dostępnego serwera. | Zainstaluj serwer wskazany w `fix` albo zezwól na niego w `code.servers_allow`. |
| `E_EXTERNAL_FAILED` | Przy `source: lsp` serwer zakończył się awarią. | Zobacz komunikat, który kończy się ostatnim wierszem zapisanym przez serwer na standardowe wyjście błędów. |
| `E_TIMEOUT` | Przy `source: lsp` serwer nie odpowiedział w czasie `code.timeout`. | Zwiększ `code.timeout` albo użyj `source: auto`. |
| `E_CANCELLED` | Żądanie zostało anulowane. | Uruchom polecenie ponownie. |

## Powiązane {#related}

`code.references`, `code.type_definition`, `code.symbols`, `code.languages`.
