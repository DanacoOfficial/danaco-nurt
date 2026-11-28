---
command: notebook.export
group: notebook
summary: Eksportuje notatnik do nowego pliku skryptu, Markdown lub tekstu.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# notebook.export

## Składnia {#synopsis}

```text
notebook.export <path> to: <path> format: <enum(script|markdown|text)>
```

## Opis {#description}

`notebook.export` zapisuje notatnik `path` do nowego pliku `to`:

| `format` | Zawartość |
|---|---|
| `script` | Skrypt w języku jądra w formacie percent: każda komórka zaczyna się od komentarza-znacznika `%%`; komórki Markdown i raw stają się komentarzami ze znacznikiem `[markdown]` lub `[raw]`. |
| `markdown` | Komórki Markdown bez zmian, komórki kodu jako bloki ogrodzone w języku jądra, a po każdej z nich ogrodzone bloki `text` z tekstem jej wyników; komórki raw bez zmian. |
| `text` | Źródła wszystkich komórek. |

Komórki są rozdzielone pustym wierszem, a plik kończy się jednym złamaniem
wiersza; notatnik bez komórek daje pusty plik. Język pochodzi z
`metadata.language_info.name`, a w przeciwnym razie z
`metadata.kernelspec.language`; znacznikiem komentarza skryptu jest `#`, `//`,
`--`, `%` lub `;;` zależnie od języka. Obrazy i inne wyniki binarne są
pomijane. Sam notatnik nie jest zmieniany.

Rekord `effect` zawiera liczniki `files_created` i `cells_exported`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik notatnika. |
| `to` | `path` | wymagany | Plik do utworzenia. |
| `format` | `enum(script\|markdown\|text)` | wymagany | Format eksportowanego pliku. |

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

## Przykłady {#examples}

Przykłady używają notatnika `/home/user/sales.ipynb` w nbformat 4.5 z trzema
komórkami: komórką Markdown `intro` (`# Sales`), komórką kodu `load`, która
odczytuje `sales.csv` do `df` i ma wynik strumieniowy `12 rows`, oraz komórką
kodu `plot` z wynikiem w postaci obrazu.

Eksport notatnika jako skryptu Python:

```nurt example
notebook.export "/home/user/sales.ipynb" to: "/home/user/sales.py" format: script
```

```text expected
# %% [markdown]
# # Sales
# Monthly totals.

# %%
import pandas as pd
df = pd.read_csv("sales.csv")
print(len(df), "rows")

# %%
df.plot(x="month", y="total")
```

Eksport notatnika jako Markdown z tekstem wyników:

```nurt example
notebook.export "/home/user/sales.ipynb" to: "/home/user/sales.md" format: markdown
```

Eksport źródeł wszystkich komórek jako tekstu:

```nurt example
notebook.export "/home/user/sales.ipynb" to: "/home/user/sales.txt" format: text
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | `path` nie istnieje | Sprawdź ścieżkę. |
| `E_PATH_RELATIVE` | `path` jest względna i nie ustawiono katalogu bazowego | Podaj ścieżkę bezwzględną albo ustaw opcję `base`. |
| `E_FORMAT` | plik nie jest poprawnym JSON, nie jest w UTF-8, nie jest w nbformat 4 albo ma powtórzone identyfikatory komórek | Napraw plik, przekonwertuj go do nbformat 4 albo uruchom `notebook.normalize` dla powtórzonych identyfikatorów. |
| `E_LIMIT` | plik jest większy niż limit tekstu | Zwiększ limit albo podziel notatnik. |
| `E_EXISTS` | `to` już istnieje | Wyeksportuj do nowego pliku albo najpierw usuń istniejący. |
| `E_NOT_FOUND` | katalog z `to` nie istnieje | Najpierw utwórz katalog. |
| `E_UNKNOWN_VALUE` | `format` jest nieznany | Użyj `script`, `markdown` lub `text`. |

## Powiązane {#related}

`notebook.cells` wyświetla komórki; `read` pokazuje notatnik jako `content` wraz
z jego komórkami. `help skill: "notebooks"` opisuje pracę z notatnikami.
