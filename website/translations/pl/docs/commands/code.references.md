---
command: code.references
group: code
summary: Znajduje odwołania do symbolu w danym położeniu, o danej nazwie lub z wejścia.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.references

## Składnia {#synopsis}

```text
code.references <path> [name: value]...
symbol |> code.references [name: value]...
```

## Opis {#description}

Znajduje odwołania do identyfikatora w danym położeniu pliku `path` albo do
definicji wskazanej przez `symbol`. W postaci etapu każdy wejściowy rekord
`symbol` jest celem, w położeniu jego nazwy.

Najpierw odpowiada serwer języka. Przy `source: auto` bez użytecznego serwera
analiza składniowa znajduje wystąpienia nazwy w kodzie w plikach projektu
w tym samym języku; wystąpienia w komentarzach i ciągach znaków nie są
odwołaniami. Wynik zawiera wtedy notatkę `semantic_fallback` z przyczyną.

Każde odwołanie ma rolę:

| Rola | Znaczenie |
|---|---|
| `definition` | Definicja symbolu. |
| `import` | Wiersz importu lub dołączenia. |
| `test` | Użycie w pliku testowym (`tests/`, `test_*.py`, `*_test.go`, `*.spec.ts` i podobne). |
| `reference` | Każde inne użycie. |

Rekordy nie zawierają powtórzeń i są uporządkowane według ścieżki, wiersza
i kolumny; liczba rekordów każdej roli jest liczbą tej roli.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik źródłowy w postaci źródła. |
| `line` | `int` | brak | Wiersz symbolu, liczony od 1. |
| `column` | `int` | brak | Kolumna symbolu, liczona od 1. |
| `symbol` | `text` | brak | Nazwa symbolu, opcjonalnie kwalifikowana jako Container.name, zamiast położenia. |
| `include_definition` | `bool` | `true` | Uwzględnia definicję wśród odwołań. |
| `scope` | `enum(project\|file)` | `project` | Przeszukuje cały projekt albo tylko plik. |
| `language` | `text` | `"auto"` | Język programowania; auto wykrywa go na podstawie rozszerzenia pliku. |
| `root` | `path` | wykrywany na podstawie położenia pliku | Katalog główny projektu dla serwera języka. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| `source` | `none` | `reference` |
| `stage` | `symbol` | `reference` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `language_server` |
| Rozwiązanie zapasowe | `builtin` |

## Przykłady {#examples}

Znalezienie każdego odwołania do funkcji.

```nurt example
code.references "/srv/app/src/shapes.py" symbol: "scale"
```

Wyświetlenie użyć symbolu w plikach testowych.

```nurt example
code.references "/srv/app/src/shapes.py" symbol: "scale" include_definition: false |> filter role == "test"
```

Znalezienie odwołań do każdej eksportowanej klasy katalogu.

```nurt example
code.symbols path: ["/srv/app/src"] kind: class exported_only: true |> code.references scope: project
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | W postaci źródła nie podano ani `line`, ani `symbol`. | Podaj `line` lub `symbol`. |
| `E_INVALID_VALUE` | `scope`, `source` lub położenie jest niepoprawne. | Sprawdź parametry. |
| `E_TYPE` | Rekord wejściowy nie jest rekordem `symbol`. | Przekaż potokiem wynik `code.symbols` lub `code.definition`. |
| `E_SYMBOL_NOT_FOUND` | `symbol` nie ma definicji w pliku. | Sprawdź nazwę za pomocą `code.symbols`. |
| `E_SYMBOL_AMBIGUOUS` | `symbol` ma w pliku kilka definicji. | Zakwalifikuj nazwę jako `Container.name` albo podaj `line` i `column`. |
| `E_UNSUPPORTED_LANGUAGE` | Potrzebna jest analiza składniowa, a dla pliku nie ma dostępnej gramatyki. | Podaj `language`. |
| `E_EXTERNAL_MISSING` | Przy `source: lsp` dla języka nie ma dostępnego serwera. | Zainstaluj serwer wskazany w `fix`. |
| `E_EXTERNAL_FAILED` | Przy `source: lsp` serwer zakończył się awarią. | Zobacz komunikat serwera. |
| `E_TIMEOUT` | Przy `source: lsp` serwer nie odpowiedział w czasie `code.timeout`. | Zwiększ `code.timeout` albo użyj `source: auto`. |

## Powiązane {#related}

`code.definition`, `code.call_hierarchy`, `code.rename`, `code.symbols`.
