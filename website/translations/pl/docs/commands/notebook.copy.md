---
command: notebook.copy
group: notebook
summary: Kopiuje komórkę na inną pozycję.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.copy

## Składnia {#synopsis}

```text
notebook.copy <path> from: <int> to: <int>
```

## Opis {#description}

`notebook.copy` wstawia kopię komórki o indeksie `from` pod indeksem `to`, który
może być dowolną pozycją od 0 do liczby komórek. Kopia dokładnie powtarza
tekst oryginału, wraz z wynikami i metadanymi, z wyjątkiem nowego
identyfikatora, gdy notatnik używa identyfikatorów.

Rekord `effect` zawiera licznik `cells_copied` oraz, w `after`, `index` i `id`
kopii.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik notatnika. |
| `from` | `int` | wymagany | Indeks komórki do skopiowania. |
| `to` | `int` | wymagany | Indeks kopii. |

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

Skopiowanie komórki `load` bezpośrednio za nią samą:

```nurt example
notebook.copy "/home/user/sales.ipynb" from: 1 to: 2
```

Powtórzenie pierwszej komórki notatnika w nbformat 4.4, którego komórki nie
mają identyfikatorów:

```nurt example
notebook.copy "/home/user/sales.ipynb" from: 0 to: 0
```

Kopiowanie komórki, która nie istnieje, kończy się błędem `E_OUT_OF_RANGE`:

```nurt example
notebook.copy "/home/user/sales.ipynb" from: 9 to: 0
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | `path` nie istnieje | Sprawdź ścieżkę. |
| `E_PATH_RELATIVE` | `path` jest względna i nie ustawiono katalogu bazowego | Podaj ścieżkę bezwzględną albo ustaw opcję `base`. |
| `E_FORMAT` | plik nie jest poprawnym JSON, nie jest w UTF-8, nie jest w nbformat 4 albo ma powtórzone identyfikatory komórek | Napraw plik, przekonwertuj go do nbformat 4 albo uruchom `notebook.normalize` dla powtórzonych identyfikatorów. |
| `E_LIMIT` | plik jest większy niż limit tekstu | Zwiększ limit albo podziel notatnik. |
| `E_STALE` | notatnik zmienił się po odczytaniu | Odczytaj notatnik ponownie i powtórz zmianę. |
| `E_OUT_OF_RANGE` | `from` wskazuje nieistniejącą komórkę albo `to` jest większe niż liczba komórek | Podaj indeksy wyświetlone przez `notebook.cells`. |

## Powiązane {#related}

`notebook.cells` wyświetla komórki i ich identyfikatory; `notebook.ops`
stosuje kilka zmian w jednym zapisie; `read` pokazuje notatnik jako `content`
wraz z jego komórkami. `help skill: "notebooks"` opisuje pracę z notatnikami.
