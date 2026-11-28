---
command: code.type_definition
group: code
summary: Znajduje definicję typu symbolu.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.type_definition

## Składnia {#synopsis}

```text
code.type_definition <path> [name: value]...
```

## Opis {#description}

Znajduje definicję typu zmiennej, parametru, pola lub wyrażenia w danym
położeniu pliku `path` albo definicji wskazanej przez `symbol`.

Typ symbolu zna wyłącznie serwer języka, więc polecenie nie ma analizy
składniowej: bez serwera dla danego języka kończy się błędem
`E_EXTERNAL_MISSING`, a awaria serwera jest błędem polecenia. `source: syntax`
również kończy się błędem `E_EXTERNAL_MISSING`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik źródłowy. |
| `line` | `int` | brak | Wiersz symbolu, liczony od 1. |
| `column` | `int` | brak | Kolumna symbolu, liczona od 1. |
| `symbol` | `text` | brak | Nazwa symbolu, opcjonalnie kwalifikowana jako Container.name, zamiast położenia. |
| `language` | `text` | `"auto"` | Język programowania; auto wykrywa go na podstawie rozszerzenia pliku. |
| `root` | `path` | wykrywany na podstawie położenia pliku | Katalog główny projektu dla serwera języka. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `symbol` |

Pola rekordu `symbol` opisano w `code.definition`; `source` ma zawsze wartość
`lsp`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `language_server` |
| Rozwiązanie zapasowe | `builtin` |

## Przykłady {#examples}

Znalezienie typu zmiennej w danym położeniu.

```nurt example
code.type_definition "/srv/app/src/main.ts" line: 22 column: 9
```

Znalezienie typu pola wskazanego przez jego kontener.

```nurt example
code.type_definition "/srv/app/src/model.py" symbol: "Order.customer"
```

Pokazanie pliku i wiersza typu.

```nurt example
code.type_definition "/srv/app/src/main.ts" line: 22 column: 9 |> select name path line
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Nie podano ani `line`, ani `symbol`. | Podaj `line` lub `symbol`. |
| `E_INVALID_VALUE` | Plik ma mniej wierszy niż `line` albo `source` jest niepoprawne. | Sprawdź parametry. |
| `E_SYMBOL_NOT_FOUND` | `symbol` nie ma definicji w pliku. | Sprawdź nazwę za pomocą `code.symbols`. |
| `E_SYMBOL_AMBIGUOUS` | `symbol` ma w pliku kilka definicji. | Zakwalifikuj nazwę jako `Container.name`. |
| `E_EXTERNAL_MISSING` | Dla języka nie ma dostępnego serwera. | Zainstaluj serwer wskazany w `fix` albo zezwól na niego w `code.servers_allow`. |
| `E_EXTERNAL_FAILED` | Serwer zakończył się awarią. | Zobacz komunikat serwera. |
| `E_TIMEOUT` | Serwer nie odpowiedział w czasie `code.timeout`. | Zwiększ `code.timeout`. |

## Powiązane {#related}

`code.definition`, `code.hover`, `code.languages`.
