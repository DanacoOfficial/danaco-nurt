---
command: notebook.move
group: notebook
summary: Przenosi komórkę na inną pozycję.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.move

## Składnia {#synopsis}

```text
notebook.move <path> from: <int> to: <int>
```

## Opis {#description}

`notebook.move` przenosi komórkę o indeksie `from` tak, aby znalazła się pod
indeksem `to`; komórki pomiędzy przesuwają się o jedną pozycję. Oba indeksy
wskazują istniejące komórki. Przeniesiona komórka i każda inna komórka
zachowują swój tekst; zmienia się tylko kolejność. Przeniesienie komórki pod
jej własny indeks niczego nie zmienia.

Rekord `effect` zawiera licznik `cells_moved`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik notatnika. |
| `from` | `int` | wymagany | Bieżący indeks komórki. |
| `to` | `int` | wymagany | Nowy indeks komórki. |

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

Przeniesienie ostatniej komórki na początek:

```nurt example
notebook.move "/home/user/sales.ipynb" from: 2 to: 0
```

Zamiana miejscami dwóch pierwszych komórek:

```nurt example
notebook.move "/home/user/sales.ipynb" from: 0 to: 1
```

Przeniesienie poza ostatnią komórkę kończy się błędem `E_OUT_OF_RANGE` i niczego nie zmienia:

```nurt example
notebook.move "/home/user/sales.ipynb" from: 0 to: 3
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | `path` nie istnieje | Sprawdź ścieżkę. |
| `E_PATH_RELATIVE` | `path` jest względna i nie ustawiono katalogu bazowego | Podaj ścieżkę bezwzględną albo ustaw opcję `base`. |
| `E_FORMAT` | plik nie jest poprawnym JSON, nie jest w UTF-8, nie jest w nbformat 4 albo ma powtórzone identyfikatory komórek | Napraw plik, przekonwertuj go do nbformat 4 albo uruchom `notebook.normalize` dla powtórzonych identyfikatorów. |
| `E_LIMIT` | plik jest większy niż limit tekstu | Zwiększ limit albo podziel notatnik. |
| `E_STALE` | notatnik zmienił się po odczytaniu | Odczytaj notatnik ponownie i powtórz zmianę. |
| `E_OUT_OF_RANGE` | `from` lub `to` wskazuje nieistniejącą komórkę | Podaj indeksy wyświetlone przez `notebook.cells`; `expected` pokazuje poprawny zakres. |

## Powiązane {#related}

`notebook.cells` wyświetla komórki i ich identyfikatory; `notebook.ops`
stosuje kilka zmian w jednym zapisie; `read` pokazuje notatnik jako `content`
wraz z jego komórkami. `help skill: "notebooks"` opisuje pracę z notatnikami.
