---
command: notebook.clear_outputs
group: notebook
summary: Czyści wyniki komórek notatnika.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.clear_outputs

## Składnia {#synopsis}

```text
notebook.clear_outputs <path>
```

## Opis {#description}

`notebook.clear_outputs` ustawia `outputs` na pustą listę, a `execution_count`
na `null` w wybranych komórkach kodu: domyślnie w każdej komórce, w przeciwnym
razie w komórkach, których indeks leży w zakresie `index`, albo w komórkach o
identyfikatorach `id`. Komórki Markdown i raw nie mają wyników i nie są
zmieniane; komórka kodu, która już nie ma wyników ani licznika, zachowuje swój
tekst. Notatnik bez wyników nie jest zapisywany.

Rekord `effect` zawiera licznik `cells_cleared`, czyli liczbę komórek, które
się zmieniły.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik notatnika. |
| `index` | `range` | `all` | Komórki do wyczyszczenia według indeksu, liczonego od 0. |
| `id` | `list<text>` | brak | Komórki do wyczyszczenia według identyfikatora. |

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

Wyczyszczenie wyników każdej komórki:

```nurt example
notebook.clear_outputs "/home/user/sales.ipynb"
```

Wyczyszczenie wyników komórki o indeksie 1:

```nurt example
notebook.clear_outputs "/home/user/sales.ipynb" index: 1
```

Wyczyszczenie wyniku obrazowego komórki `plot`:

```nurt example
notebook.clear_outputs "/home/user/sales.ipynb" id: ["plot"]
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | `path` nie istnieje | Sprawdź ścieżkę. |
| `E_PATH_RELATIVE` | `path` jest względna i nie ustawiono katalogu bazowego | Podaj ścieżkę bezwzględną albo ustaw opcję `base`. |
| `E_FORMAT` | plik nie jest poprawnym JSON, nie jest w UTF-8, nie jest w nbformat 4 albo ma powtórzone identyfikatory komórek | Napraw plik, przekonwertuj go do nbformat 4 albo uruchom `notebook.normalize` dla powtórzonych identyfikatorów. |
| `E_LIMIT` | plik jest większy niż limit tekstu | Zwiększ limit albo podziel notatnik. |
| `E_STALE` | notatnik zmienił się po odczytaniu | Odczytaj notatnik ponownie i powtórz zmianę. |
| `E_OUT_OF_RANGE` | granica `index` wskazuje nieistniejącą komórkę | Podaj indeksy wyświetlone przez `notebook.cells`. |
| `E_ARG` | żadna komórka nie ma identyfikatora z `id` | Użyj identyfikatorów wyświetlonych przez `notebook.cells`. |
| `E_PARAM_CONFLICT` | podano jednocześnie `index` i `id` | Podaj jeden z nich. |

## Powiązane {#related}

`notebook.cells` wyświetla komórki i ich identyfikatory; `notebook.ops`
stosuje kilka zmian w jednym zapisie; `read` pokazuje notatnik jako `content`
wraz z jego komórkami. `help skill: "notebooks"` opisuje pracę z notatnikami.
