---
command: code.hover
group: code
summary: Pokazuje sygnaturę i dokumentację symbolu w danym położeniu.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.hover

## Składnia {#synopsis}

```text
code.hover <path> [name: value]...
```

## Opis {#description}

Pokazuje sygnaturę i dokumentację identyfikatora w danym położeniu pliku
`path` albo definicji wskazanej przez `symbol`.

Odpowiedź serwera języka jest skracana: sygnaturą jest pierwszy blok kodu
odpowiedzi, przy czym sygnatura zapisana w kilku wierszach jest łączona
w jeden wiersz, a dokumentacją jest pierwsze zdanie pozostałego tekstu. Przy
`verbosity: detailed` w opcjach żądania dokumentacją jest cała odpowiedź
serwera.

Przy `source: auto` bez użytecznego serwera albo gdy serwer nie ma informacji,
definicja jest znajdowana analizą składniową; sygnaturą jest wtedy wiersz jej
deklaracji, a dokumentacją pierwsze zdanie docstringu pod nazwą albo wierszy
komentarza nad definicją. Wynik zawiera wtedy notatkę `semantic_fallback`
z przyczyną. Przy `source: lsp` serwer bez informacji nie daje żadnego rekordu.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik źródłowy. |
| `line` | `int` | brak | Wiersz symbolu, liczony od 1; wymagany, chyba że podano symbol. |
| `column` | `int` | brak | Kolumna symbolu, liczona od 1; wymagana, chyba że podano symbol. |
| `symbol` | `text` | brak | Nazwa symbolu, opcjonalnie kwalifikowana jako Container.name, zamiast położenia. |
| `language` | `text` | `"auto"` | Język programowania; auto wykrywa go na podstawie rozszerzenia pliku. |
| `root` | `path` | wykrywany na podstawie położenia pliku | Katalog główny projektu dla serwera języka. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `hover` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `language_server` |
| Rozwiązanie zapasowe | `builtin` |

## Przykłady {#examples}

Pokazanie sygnatury funkcji wywołanej w wierszu 5.

```nurt example
code.hover "/srv/app/src/main.py" line: 5 column: 12
```

```text expected
signature: def scale(value) -> int
```

Pokazanie sygnatury i dokumentacji klasy.

```nurt example
code.hover "/srv/app/src/shapes.py" symbol: "Circle" |> select signature documentation
```

Odczytanie dokumentacji symbolu po nazwie.

```nurt example
code.hover "/srv/app/src/shapes.py" symbol: "scale"
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

`code.definition`, `code.symbols`, `code.outline`.
