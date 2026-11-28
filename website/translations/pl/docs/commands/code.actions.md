---
command: code.actions
group: code
summary: Wyświetla akcje kodu oferowane przez serwer języka dla zakresu wierszy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.actions

## Składnia {#synopsis}

```text
code.actions <path> line: <int> [name: value]...
```

## Opis {#description}

Wyświetla akcje kodu (szybkie poprawki, refaktoryzacje, akcje na źródle),
które serwer języka oferuje dla wierszy od `line` do `end_line` pliku `path`.
Zakres zaczyna się w kolumnie `column` pierwszego wiersza albo na jego
początku, a kończy na końcu ostatniego wiersza. Diagnostyki opublikowane przez
serwer dla tego zakresu są wysyłane jako kontekst żądania.

Polecenie wyświetla akcje i żadnej nie stosuje. Akcje kodu oferuje wyłącznie
serwer języka: bez serwera dla danego języka polecenie kończy się błędem
`E_EXTERNAL_MISSING`, a awaria serwera jest błędem polecenia.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik źródłowy. |
| `line` | `int` | wymagany | Pierwszy wiersz zakresu, liczony od 1. |
| `end_line` | `int` | brak | Ostatni wiersz zakresu. |
| `column` | `int` | brak | Kolumna w pierwszym wierszu, liczona od 1. |
| `language` | `text` | `"auto"` | Język programowania; auto wykrywa go na podstawie rozszerzenia pliku. |
| `root` | `path` | wykrywany na podstawie położenia pliku | Katalog główny projektu dla serwera języka. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `code_action` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `language_server` |
| Rozwiązanie zapasowe | `builtin` |

## Przykłady {#examples}

Wyświetlenie akcji dla jednego wiersza.

```nurt example
code.actions "/srv/app/src/main.py" line: 14
```

Wyświetlenie refaktoryzacji oferowanych dla bloku wierszy.

```nurt example
code.actions "/srv/app/src/main.ts" line: 20 end_line: 34 |> filter kind == "refactor.extract"
```

Wyświetlenie szybkich poprawek przy błędzie.

```nurt example
code.actions "/srv/app/src/lib.rs" line: 52 column: 17 |> filter kind == "quickfix" |> select title
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Nie podano `line`. | Podaj `line`. |
| `E_INVALID_VALUE` | `end_line` jest przed `line` albo plik ma mniej wierszy niż zakres. | Podaj zakres wewnątrz pliku. |
| `E_EXTERNAL_MISSING` | Dla języka nie ma dostępnego serwera. | Zainstaluj serwer wskazany w `fix` albo zezwól na niego w `code.servers_allow`. |
| `E_EXTERNAL_FAILED` | Serwer zakończył się awarią. | Zobacz komunikat serwera. |
| `E_TIMEOUT` | Serwer nie odpowiedział w czasie `code.timeout`. | Zwiększ `code.timeout`. |

## Powiązane {#related}

`code.diagnostics`, `code.rename`, `replace`.
