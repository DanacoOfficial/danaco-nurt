---
command: notebook.normalize
group: notebook
summary: Dodaje brakujące identyfikatory komórek i pola nbformat 4.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.normalize

## Składnia {#synopsis}

```text
notebook.normalize <path>
```

## Opis {#description}

`notebook.normalize` doprowadza notatnik `path` do nbformat 4.5:

- `nbformat_minor` przyjmuje wartość 5, gdy jest niższy, a brakujące
  `metadata` notatnika jest dodawane;
- każda komórka bez poprawnego identyfikatora albo z identyfikatorem, który ma
  już wcześniejsza komórka, otrzymuje nowy identyfikator złożony z ośmiu cyfr
  szesnastkowych;
- brakujące `metadata` i `source` są dodawane do komórek, a brakujące
  `outputs` i `execution_count` do komórek kodu;
- składowe, których nbformat nie dopuszcza dla danego typu komórki, są
  usuwane: `outputs` i `execution_count` komórek Markdown i raw oraz
  `attachments` komórek kodu.

Komórki, które nie wymagają żadnej z tych zmian, zachowują swój tekst. Drugie
wywołanie niczego nie zmienia i niczego nie zapisuje. Pozostałe polecenia
notatnika nigdy nie dodają identyfikatorów ani składowych do komórek, których
nie zmieniają.

Rekord `effect` zawiera licznik `cells_changed`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik notatnika. |

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

Dodanie identyfikatorów do notatnika w nbformat 4.4, którego komórki ich nie
mają:

```nurt example
notebook.normalize "/home/user/sales.ipynb"
```

Nadanie nowego identyfikatora drugiej z dwóch komórek, które mają wspólny
identyfikator `load`:

```nurt example
notebook.normalize "/home/user/sales.ipynb"
```

Dodanie `outputs` i `execution_count` do komórki kodu, której brakuje tych pól:

```nurt example
notebook.normalize "/home/user/sales.ipynb"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | `path` nie istnieje | Sprawdź ścieżkę. |
| `E_PATH_RELATIVE` | `path` jest względna i nie ustawiono katalogu bazowego | Podaj ścieżkę bezwzględną albo ustaw opcję `base`. |
| `E_FORMAT` | plik nie jest poprawnym JSON, nie jest w UTF-8, nie jest w nbformat 4 albo ma powtórzone identyfikatory komórek | Napraw plik, przekonwertuj go do nbformat 4 albo uruchom `notebook.normalize` dla powtórzonych identyfikatorów. |
| `E_LIMIT` | plik jest większy niż limit tekstu | Zwiększ limit albo podziel notatnik. |
| `E_STALE` | notatnik zmienił się po odczytaniu | Odczytaj notatnik ponownie i powtórz zmianę. |

## Powiązane {#related}

`notebook.cells` wyświetla komórki i ich identyfikatory; `notebook.ops`
stosuje kilka zmian w jednym zapisie; `read` pokazuje notatnik jako `content`
wraz z jego komórkami. `help skill: "notebooks"` opisuje pracę z notatnikami.
