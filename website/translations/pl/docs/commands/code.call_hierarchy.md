---
command: code.call_hierarchy
group: code
summary: Wyświetla wywołujących i wywoływane funkcje danej funkcji.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.call_hierarchy

## Składnia {#synopsis}

```text
code.call_hierarchy <path> [name: value]...
```

## Opis {#description}

Wyświetla funkcje, które wywołują funkcję w danym położeniu pliku `path` (lub
wskazaną przez `symbol`), funkcje przez nią wywoływane albo jedne i drugie.

Najpierw odpowiada serwer języka. Przy `source: auto` bez użytecznego serwera
hierarchię przybliża analiza składniowa: wywołującymi są funkcje i metody,
które zawierają w kodzie odwołanie do nazwy, a wywoływanymi są funkcje projektu
wywołane w treści definicji. Wywołania składowych (`object.name(...)`) nie są
wywoływanymi w analizie składniowej. Wynik zawiera wtedy notatkę
`semantic_fallback` z przyczyną.

Każdy rekord `reference` jest jednym wywołującym lub wywoływanym. Jego `text`
zawiera kierunek (`<-` dla wywołującego, `->` dla wywoływanego), rodzaj,
nazwę kwalifikowaną oraz, dla więcej niż jednego wywołania, liczbę wywołań
(`x3`).

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik źródłowy. |
| `line` | `int` | brak | Wiersz funkcji, liczony od 1. |
| `column` | `int` | brak | Kolumna funkcji, liczona od 1. |
| `symbol` | `text` | brak | Nazwa funkcji, opcjonalnie kwalifikowana jako Container.name, zamiast położenia. |
| `direction` | `enum(incoming\|outgoing\|both)` | `both` | Wywołujący, wywoływane lub jedni i drudzy. |
| `language` | `text` | `"auto"` | Język programowania; auto wykrywa go na podstawie rozszerzenia pliku. |
| `root` | `path` | wykrywany na podstawie położenia pliku | Katalog główny projektu dla serwera języka. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `reference` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `language_server` |
| Rozwiązanie zapasowe | `builtin` |

## Przykłady {#examples}

Wyświetlenie wywołujących funkcję.

```nurt example
code.call_hierarchy "/srv/app/src/shapes.py" symbol: "scale" direction: incoming
```

```text expected
<- function total x2
```

Wyświetlenie funkcji wywoływanych przez metodę.

```nurt example
code.call_hierarchy "/srv/app/src/cart.ts" symbol: "Cart.checkout" direction: outgoing
```

Wyświetlenie wywołujących i wywoływanych funkcji w innym wierszu.

```nurt example
code.call_hierarchy "/srv/app/src/main.go" line: 42
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Nie podano ani `line`, ani `symbol`. | Podaj `line` lub `symbol`. |
| `E_INVALID_VALUE` | `direction` lub `source` jest niepoprawne albo plik ma mniej wierszy niż `line`. | Sprawdź parametry. |
| `E_SYMBOL_NOT_FOUND` | `symbol` nie ma definicji w pliku. | Sprawdź nazwę za pomocą `code.symbols`. |
| `E_SYMBOL_AMBIGUOUS` | `symbol` ma w pliku kilka definicji. | Zakwalifikuj nazwę jako `Container.name`. |
| `E_UNSUPPORTED_LANGUAGE` | Potrzebna jest analiza składniowa, a dla pliku nie ma dostępnej gramatyki. | Podaj `language`. |
| `E_EXTERNAL_MISSING` | Przy `source: lsp` dla języka nie ma dostępnego serwera. | Zainstaluj serwer wskazany w `fix`. |
| `E_EXTERNAL_FAILED` | Przy `source: lsp` serwer zakończył się awarią. | Zobacz komunikat serwera. |
| `E_TIMEOUT` | Przy `source: lsp` serwer nie odpowiedział w czasie `code.timeout`. | Zwiększ `code.timeout` albo użyj `source: auto`. |

## Powiązane {#related}

`code.references`, `code.definition`, `code.profile`.
