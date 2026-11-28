---
command: notebook.cells
group: notebook
summary: Wyświetla komórki notatnika jako zarys lub z pełną zawartością.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.cells

## Składnia {#synopsis}

```text
notebook.cells <path> [name: value]...
```

## Opis {#description}

`notebook.cells` produkuje jeden rekord `cell` dla każdej wybranej komórki
notatnika `path`, w kolejności komórek. Każdy rekord zawiera `index` (od 0),
`id` (nbformat 4.5 i nowsze), `cell_type`, `source`, `execution_count` komórek
kodu oraz `etag`, czyli znacznik encji pliku.

Bez `index`, `id` i `find` każda komórka jest wyświetlana jako zarys: `source`
jest pierwszym niepustym wierszem, a `metadata` jest pomijane. Z którymkolwiek
z nich wybrane komórki są wyświetlane w całości, z całym źródłem i metadanymi.
Jawnie podany `view` zastępuje ten wybór.

`index` i `id` wybierają komórki łącznie: komórka jest wyświetlana, gdy jej
indeks leży w zakresie albo jej identyfikator jest na liście. Początek `index`
musi wskazywać istniejącą komórkę; koniec może leżeć za ostatnią komórką.
`cell_type` zachowuje tylko komórki tego typu. `find` zachowuje tylko komórki,
w których występuje tekst lub wyrażenie regularne, w źródle albo, z `scope:
outputs`, w tekście wyników. Wyrażenia regularne używają dialektu `search`.

`outputs` ustawia poziom szczegółowości wyników komórek kodu: `summary` dodaje
`outputs_summary` z liczbą i rodzajami wyników, na przykład
`1 output: stream stdout`; `text` wyświetla także każdy wynik zredukowany do
jego typu, nazwy strumienia i tekstu, bez obrazów i innych danych binarnych;
`full` wyświetla wyniki w postaci zapisanej.

Notatnik bez komórek nie produkuje rekordów.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik notatnika. |
| `index` | `range` | brak | Komórki do zwrócenia według indeksu, liczonego od 0. |
| `id` | `list<text>` | brak | Komórki do zwrócenia według identyfikatora. |
| `cell_type` | `enum(code\|markdown\|raw)` | brak | Tylko komórki tego typu. |
| `outputs` | `enum(none\|summary\|text\|full)` | `none` | Poziom szczegółowości zwracanych wyników komórek. |
| `find` | `text \| regex` | brak | Tylko komórki zawierające ten tekst lub wzorzec. |
| `scope` | `enum(source\|outputs)` | `source` | Część komórki przeszukiwana przez find. |
| `view` | `enum(outline\|full)` | `outline` | Zarys lub pełna zawartość, gdy nie wybrano komórek. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `cell` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Przykłady używają notatnika `/home/user/sales.ipynb` w nbformat 4.5 z trzema
komórkami: komórką Markdown `intro` (`# Sales`), komórką kodu `load`, która
odczytuje `sales.csv` do `df` i ma wynik strumieniowy `12 rows`, oraz komórką
kodu `plot` z wynikiem w postaci obrazu.

Wyświetlenie zarysu każdej komórki:

```nurt example
notebook.cells "/home/user/sales.ipynb"
```

```text expected
0  markdown  intro  # Sales
1  code      load   import pandas as pd
2  code      plot   df.plot(x="month", y="total")
```

Wyświetlenie jednej komórki w całości z tekstem jej wyników:

```nurt example
notebook.cells "/home/user/sales.ipynb" index: 1 outputs: text
```

Wyświetlenie komórek, których źródło wywołuje funkcję `read_`:

```nurt example
notebook.cells "/home/user/sales.ipynb" find: /read_\w+/
```

Podsumowanie wyników komórek kodu:

```nurt example
notebook.cells "/home/user/sales.ipynb" cell_type: code outputs: summary
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | `path` nie istnieje | Sprawdź ścieżkę. |
| `E_PATH_RELATIVE` | `path` jest względna i nie ustawiono katalogu bazowego | Podaj ścieżkę bezwzględną albo ustaw opcję `base`. |
| `E_FORMAT` | plik nie jest poprawnym JSON, nie jest w UTF-8, nie jest w nbformat 4 albo ma powtórzone identyfikatory komórek | Napraw plik, przekonwertuj go do nbformat 4 albo uruchom `notebook.normalize` dla powtórzonych identyfikatorów. |
| `E_LIMIT` | plik jest większy niż limit tekstu | Zwiększ limit albo podziel notatnik. |
| `E_OUT_OF_RANGE` | początek `index` leży za ostatnią komórką | Najpierw wyświetl zarys, aby zobaczyć poprawne indeksy. |
| `E_ARG` | żadna komórka nie ma identyfikatora z `id` | Użyj identyfikatora z zarysu; komunikat wskazuje najbliższe. |
| `E_REGEX` | `find` nie jest poprawnym wyrażeniem regularnym | Popraw wzorzec w zgłoszonym miejscu. |
| `E_UNKNOWN_VALUE` | wartość `cell_type`, `outputs`, `scope` lub `view` jest nieznana | Użyj jednej z wymienionych wartości. |

## Powiązane {#related}

`read` pokazuje notatnik jako `content` wraz z jego komórkami;
`notebook.replace`, `notebook.insert` i `notebook.ops` zmieniają komórki.
`help skill: "notebooks"` opisuje pracę z notatnikami.
