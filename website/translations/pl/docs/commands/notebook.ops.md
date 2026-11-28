---
command: notebook.ops
group: notebook
summary: Stosuje listę operacji na komórkach w jednym zapisie.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.ops

## Składnia {#synopsis}

```text
notebook.ops <path> changes: <list<map<any>>>
```

## Opis {#description}

`notebook.ops` stosuje operacje z `changes` po kolei, każdą do wyniku
poprzednich, i zapisuje notatnik jeden raz. Gdy operacja zawiedzie, nic nie
zostaje zmienione: błąd wskazuje operację w `details.change`, liczoną od 0.

Każda operacja jest mapą ze składową `action` i parametrami odpowiadającego
jej polecenia:

| `action` | Składowe | Jak |
|---|---|---|
| `insert` | `cell_type`, `source`, `index` lub `after_id` | `notebook.insert` |
| `replace` | `index` lub `id`, `source`, `cell_type`, `find`, `with`, `whole_word`, `reset_outputs` | `notebook.replace` |
| `move` | `index` lub `id` komórki, `to` | `notebook.move` |
| `copy` | `index` lub `id` komórki, `to` (bezpośrednio za oryginałem, gdy brakuje) | `notebook.copy` |
| `remove` | `index` (indeks albo zakres, taki jak `"1..3"`) lub `id` (jeden albo lista) | `notebook.remove` |
| `clear_outputs` | `index` lub `id`; każda komórka, gdy oba brakują | `notebook.clear_outputs` |

Składowa, która nie dotyczy danej akcji, jest błędem. `find` może być tekstem
albo wyrażeniem regularnym.

Rekord `effect` zawiera liczniki `operations`, `cells_inserted`,
`cells_changed`, `cells_moved`, `cells_copied`, `cells_removed`,
`cells_cleared` i `replacements`, każdy wtedy, gdy nie jest zerem, oraz w
`after` listę z `change`, `index` i `id` każdej wstawionej lub skopiowanej
komórki.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik notatnika. |
| `changes` | `list<map<any>>` | wymagany | Operacje stosowane razem; wszystkie się powiodą albo żadna. |

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

Wstawienie notatki po tytule i usunięcie wykresu w jednym zapisie:

```nurt example
notebook.ops "/home/user/sales.ipynb" changes: [{action: "insert", after_id: "intro", cell_type: "markdown", source: "Data source: sales.csv"}, {action: "remove", id: "plot"}]
```

Zmiana nazwy zmiennej w jednej komórce i wyczyszczenie każdego wyniku:

```nurt example
notebook.ops "/home/user/sales.ipynb" changes: [{action: "replace", id: "load", find: "df", with: "sales", whole_word: true}, {action: "clear_outputs"}]
```

Operacja, która zawodzi, pozostawia notatnik bez zmian; tutaj druga operacja
wskazuje komórkę, która nie istnieje:

```nurt example
notebook.ops "/home/user/sales.ipynb" changes: [{action: "remove", index: 0}, {action: "move", index: 9, to: 0}]
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | `path` nie istnieje | Sprawdź ścieżkę. |
| `E_PATH_RELATIVE` | `path` jest względna i nie ustawiono katalogu bazowego | Podaj ścieżkę bezwzględną albo ustaw opcję `base`. |
| `E_FORMAT` | plik nie jest poprawnym JSON, nie jest w UTF-8, nie jest w nbformat 4 albo ma powtórzone identyfikatory komórek | Napraw plik, przekonwertuj go do nbformat 4 albo uruchom `notebook.normalize` dla powtórzonych identyfikatorów. |
| `E_LIMIT` | plik jest większy niż limit tekstu | Zwiększ limit albo podziel notatnik. |
| `E_STALE` | notatnik zmienił się po odczytaniu | Odczytaj notatnik ponownie i powtórz zmianę. |
| `E_ARG` | operacja ma nieznaną akcję, składową, która nie dotyczy akcji, brakującą lub błędnie otypowaną składową albo wskazuje nieznany identyfikator; albo `changes` jest puste | Popraw operację wskazaną w `details.change`. |
| `E_OUT_OF_RANGE` | operacja wskazuje indeks poza notatnikiem | Popraw operację wskazaną w `details.change`; indeksy odnoszą się do wyniku wcześniejszych operacji. |
| `E_NO_MATCH` | `find` operacji nie występuje w żadnej wybranej komórce | Sprawdź tekst lub komórkę tej operacji. |
| `E_REGEX` | wyrażenie regularne lub szablon zastępowania operacji jest nieprawidłowy | Popraw wzorzec lub szablon. |
| `E_TYPE` | `changes` nie jest listą | Podaj listę map. |

## Powiązane {#related}

`notebook.cells` wyświetla komórki i ich identyfikatory; `notebook.ops`
stosuje kilka zmian w jednym zapisie; `read` pokazuje notatnik jako `content`
wraz z jego komórkami. `help skill: "notebooks"` opisuje pracę z notatnikami.
