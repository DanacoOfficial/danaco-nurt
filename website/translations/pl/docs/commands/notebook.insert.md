---
command: notebook.insert
group: notebook
summary: Wstawia nową komórkę do notatnika.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.insert

## Składnia {#synopsis}

```text
notebook.insert <path> cell_type: <enum(code|markdown|raw)> source: <text>
```

## Opis {#description}

`notebook.insert` dodaje jedną komórkę do notatnika `path`: pod `index`,
bezpośrednio za komórką o identyfikatorze `after_id` albo za ostatnią komórką,
gdy nie podano żadnego z nich. Komórka o indeksie `index` i komórki po niej
przesuwają się o jedną pozycję w dół.

Nowa komórka ma puste `metadata`; komórka kodu ma także `execution_count:
null` i nie ma wyników. Jej `source` jest zapisywane w postaci używanej przez
pozostałe komórki, jako lista wierszy lub pojedynczy ciąg znaków. Gdy notatnik
jest w nbformat 4.5 lub nowszym albo jego komórki mają już identyfikatory,
nowa komórka otrzymuje nowy identyfikator złożony z ośmiu cyfr szesnastkowych,
którego nie ma żadna inna komórka.

Rekord `effect` zawiera licznik `cells_inserted` oraz, w `after`, `index` i
`id` nowej komórki.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik notatnika. |
| `cell_type` | `enum(code\|markdown\|raw)` | wymagany | Typ nowej komórki. |
| `source` | `text` | wymagany | Źródło nowej komórki. |
| `index` | `int` | koniec notatnika | Pozycja nowej komórki, liczona od 0. |
| `after_id` | `text` | brak | Wstaw za komórką o tym identyfikatorze. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

Komórki i składowe, których zmiana nie dotyczy, zachowują swoje bajty:
kolejność kluczy, wcięcia, zapis liczb, sekwencje ucieczki i układ pliku są
zachowane, więc różnica pliku pokazuje tylko zmienione komórki. Wywołanie,
które pozostawia notatnik bez zmian, niczego nie zapisuje i nie tworzy wpisu
dziennika; jego rekord `effect` ma `targets: 0`.

## Przykłady {#examples}

Przykłady używają notatnika `/home/user/sales.ipynb` w nbformat 4.5 z trzema
komórkami: komórką Markdown `intro` (`# Sales`), komórką kodu `load`, która
odczytuje `sales.csv` do `df` i ma wynik strumieniowy `12 rows`, oraz komórką
kodu `plot` z wynikiem w postaci obrazu.

Dopisanie komórki Markdown na końcu:

```nurt example
notebook.insert "/home/user/sales.ipynb" cell_type: markdown source: "## Totals"
```

Wstawienie komórki kodu przed komórką `load`, pod indeksem 1:

```nurt example
notebook.insert "/home/user/sales.ipynb" index: 1 cell_type: code source: "import numpy as np"
```

Wstawienie komórki kodu za komórką `load`:

```nurt example
notebook.insert "/home/user/sales.ipynb" after_id: "load" cell_type: code source: "df.describe()"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | `path` nie istnieje | Sprawdź ścieżkę. |
| `E_PATH_RELATIVE` | `path` jest względna i nie ustawiono katalogu bazowego | Podaj ścieżkę bezwzględną albo ustaw opcję `base`. |
| `E_FORMAT` | plik nie jest poprawnym JSON, nie jest w UTF-8, nie jest w nbformat 4 albo ma powtórzone identyfikatory komórek | Napraw plik, przekonwertuj go do nbformat 4 albo uruchom `notebook.normalize` dla powtórzonych identyfikatorów. |
| `E_LIMIT` | plik jest większy niż limit tekstu | Zwiększ limit albo podziel notatnik. |
| `E_STALE` | notatnik zmienił się po odczytaniu | Odczytaj notatnik ponownie i powtórz zmianę. |
| `E_OUT_OF_RANGE` | `index` jest ujemny albo większy niż liczba komórek | Podaj indeks od 0 do liczby komórek. |
| `E_ARG` | żadna komórka nie ma identyfikatora `after_id` | Użyj identyfikatora wyświetlonego przez `notebook.cells`. |
| `E_PARAM_CONFLICT` | podano jednocześnie `index` i `after_id` | Podaj jeden z nich. |
| `E_UNKNOWN_VALUE` | `cell_type` jest nieznany | Użyj `code`, `markdown` lub `raw`. |

## Powiązane {#related}

`notebook.cells` wyświetla komórki i ich identyfikatory; `notebook.ops`
stosuje kilka zmian w jednym zapisie; `read` pokazuje notatnik jako `content`
wraz z jego komórkami. `help skill: "notebooks"` opisuje pracę z notatnikami.
