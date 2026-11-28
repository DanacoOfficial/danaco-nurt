---
command: notebook.replace
group: notebook
summary: Zastępuje źródło lub typ komórki albo tekst w źródłach komórek.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.replace

## Składnia {#synopsis}

```text
notebook.replace <path> [name: value]...
```

## Opis {#description}

Pierwsza postać zmienia jedną komórkę wskazaną przez `index` lub `id`: `source`
zastępuje całe jej źródło, `cell_type` zmienia jej typ, a `reset_outputs`
czyści jej wyniki i licznik wykonania. Wymagane jest co najmniej jedno z nich.

Druga postać zastępuje każde dopasowanie `find` tekstem `with` w źródle
wskazanej komórki albo, bez `index` i `id`, w źródłach wszystkich komórek.
`whole_word` dopasowuje tylko tam, gdzie tekst nie jest poprzedzony ani
następowany literą, cyfrą ani `_`, więc `df` nie pasuje wewnątrz `df2`. Przy
wyrażeniu regularnym `with` jest szablonem: `$1` lub `${1}` wstawia grupę
przechwytującą według numeru, `$0` całe dopasowanie, a `$$` znak `$`. Z
`reset_outputs` komórki, w których zastąpiono tekst, tracą swoje wyniki.
Wymagane jest co najmniej jedno dopasowanie.

`source` i `find` wykluczają się wzajemnie. Źródło zachowuje postać zapisaną w
komórce, jako lista wierszy lub pojedynczy ciąg znaków. Zmiana `cell_type`
dodaje `outputs` i `execution_count`, gdy komórka staje się komórką kodu, a w
przeciwnym razie usuwa je wraz z `attachments` komórek kodu. Wyniki pozostają,
chyba że ustawiono `reset_outputs`.

Gdy podano jednocześnie `index` i `id`, muszą wskazywać tę samą komórkę.

Rekord `effect` zawiera liczniki `cells_changed` oraz, z `find`,
`replacements`; rekord `change` także zawiera `replacements`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik notatnika. |
| `index` | `int` | brak | Komórka do zmiany według indeksu, liczonego od 0. |
| `id` | `text` | brak | Komórka do zmiany według identyfikatora. |
| `source` | `text` | brak | Nowe źródło komórki. |
| `cell_type` | `enum(code\|markdown\|raw)` | brak | Nowy typ komórki. |
| `find` | `text \| regex` | brak | Tekst lub wzorzec do zastąpienia w źródłach komórek. |
| `with` | `text` | brak | Zamiennik tekstu dopasowanego przez find. |
| `whole_word` | `bool` | `false` | Dopasowuje find tylko jako całe słowo lub identyfikator. |
| `reset_outputs` | `bool` | `false` | Czyści wyniki zmienionych komórek. |

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

Zastąpienie źródła ostatniej komórki:

```nurt example
notebook.replace "/home/user/sales.ipynb" index: 2 source: "df.plot(x=\"month\", y=\"total\", kind=\"bar\")"
```

Zmiana nazwy zmiennej `df` w każdej komórce:

```nurt example
notebook.replace "/home/user/sales.ipynb" find: "df" with: "sales" whole_word: true
```

Przepisanie wywołania za pomocą wyrażenia regularnego w komórce `load`:

```nurt example
notebook.replace "/home/user/sales.ipynb" id: "load" find: /read_(\w+)/ with: "load_$1"
```

Wyczyszczenie wyników komórki `load`:

```nurt example
notebook.replace "/home/user/sales.ipynb" id: "load" reset_outputs: true
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | `path` nie istnieje | Sprawdź ścieżkę. |
| `E_PATH_RELATIVE` | `path` jest względna i nie ustawiono katalogu bazowego | Podaj ścieżkę bezwzględną albo ustaw opcję `base`. |
| `E_FORMAT` | plik nie jest poprawnym JSON, nie jest w UTF-8, nie jest w nbformat 4 albo ma powtórzone identyfikatory komórek | Napraw plik, przekonwertuj go do nbformat 4 albo uruchom `notebook.normalize` dla powtórzonych identyfikatorów. |
| `E_LIMIT` | plik jest większy niż limit tekstu | Zwiększ limit albo podziel notatnik. |
| `E_STALE` | notatnik zmienił się po odczytaniu | Odczytaj notatnik ponownie i powtórz zmianę. |
| `E_OUT_OF_RANGE` | `index` wskazuje nieistniejącą komórkę | Podaj indeks wyświetlony przez `notebook.cells`. |
| `E_ARG` | żadna komórka nie ma tego identyfikatora, `index` i `id` wskazują różne komórki albo argumenty są niekompletne lub niespójne | Postępuj zgodnie z komunikatem: podaj jedną komórkę, `source` albo `find` z `with`. |
| `E_NO_MATCH` | `find` nie występuje w żadnej wybranej komórce | Sprawdź tekst lub wybór komórek. |
| `E_REGEX` | `find` nie jest poprawnym wyrażeniem regularnym albo `with` odwołuje się do nieistniejącej grupy | Popraw wzorzec lub szablon; dla dosłownego `$` napisz `$$`. |

## Powiązane {#related}

`notebook.cells` wyświetla komórki i ich identyfikatory; `notebook.ops`
stosuje kilka zmian w jednym zapisie; `read` pokazuje notatnik jako `content`
wraz z jego komórkami. `help skill: "notebooks"` opisuje pracę z notatnikami.
