---
command: analyze.show
group: verify
summary: Pokazuje ustalenia wcześniejszej analizy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# analyze.show

## Składnia {#synopsis}

```text
analyze.show [<analysis_id>] [name: value]...
```

## Opis {#description}

Zwraca zapisany nagłówek `analysis` i ustalenia analizy wykonanej przez
`analyze.check`. Bez `analysis_id` używana jest najnowsza analiza. Ustalenia są
filtrowane według kontroli, pliku (ścieżka zawierająca podany tekst) i poziomu
ważności, a następnie dzielone na strony za pomocą `offset` i `max_findings`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `analysis_id` | `text` | najnowsza analiza | Wcześniejsza analiza. |
| `check` | `text` | brak | Tylko ustalenia tej kontroli. |
| `file` | `path` | brak | Tylko ustalenia w tym pliku. |
| `offset` | `int` | brak | Liczba ustaleń do pominięcia. |
| `min_severity` | `enum(error\|warning\|info)` | `info` | Najniższy poziom ważności zwracanych ustaleń. |
| `max_findings` | `int` | `100` | Maksymalna liczba zwracanych ustaleń. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `finding` |

Brak wejścia. Wyjście: nagłówek `analysis` i rekordy `finding`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Pokazanie ustaleń najnowszej analizy.

```nurt example
analyze.show
```

Pokazanie błędów lintera z jednej analizy w jednym pliku.

```nurt example
analyze.show "an-3" check: "lint" file: "/home/user/app/src/app.py" min_severity: error
```

Pokazanie drugiej strony ustaleń.

```nurt example
analyze.show offset: 100
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_UNKNOWN_ANALYSIS` | Żadna zapisana analiza nie ma tego identyfikatora; błąd wymienia kandydatów. | Użyj identyfikatora spośród kandydatów albo uruchom `analyze.check`. |

## Powiązane {#related}

`analyze.check`.
