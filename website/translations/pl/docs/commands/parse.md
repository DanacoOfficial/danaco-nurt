---
command: parse
group: text
summary: Wyodrębnia pola z tekstu za pomocą nazwanych grup wyrażenia regularnego.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# parse

## Składnia {#synopsis}

```text
line|match|content |> parse <pattern>
```

## Opis {#description}

`parse` dopasowuje `pattern` do tekstu każdego rekordu i daje rekord `row`,
którego `cells` są nazwanymi grupami dopasowania, jako tekst; grupa, która nie
uczestniczy w dopasowaniu, ma wartość `null`. Grupy są nazywane za pomocą
`(?<name>…)`. Wiersz zachowuje `path` swojego rekordu wejściowego.

| Wejście | Tekst | Wiersze |
|---|---|---|
| `line` | `text` | jeden wiersz dla pierwszego dopasowania, numerowany według wiersza |
| `match` | `line`, cały wiersz dopasowania | jeden wiersz dla pierwszego dopasowania, numerowany według wiersza |
| `content` | `text` | jeden wiersz na dopasowanie, numerowany według wiersza, w którym dopasowanie się zaczyna |

Rekordy, do których wzorzec nie pasuje, nie dają wiersza. Etap jest leniwy i
zachowuje nagłówek strumienia.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `pattern` | `regex` | wymagany | Wzorzec, którego nazwane grupy stają się polami. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `line`, `match`, `content` | `row` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `transform` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Podział wierszy dziennika na poziom i komunikat:

```nurt example
lines "/home/user/app.log" |> parse /(?<level>[A-Z]+) (?<message>.*)/
```

Odczyt ustawień z pliku środowiska:

```nurt example
read "/home/user/app.env" |> parse /^(?<key>\w+)=(?<value>.*)$/m
```

Wyodrębnienie nazwy i liczby z wierszy znalezionych przez wyszukiwanie:

```nurt example
search /port/ "/home/user/app.conf" |> parse /(?<name>\w+)\s*=\s*(?<value>\d+)/
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | wzorzec nie ma nazwanych grup | nazwij grupy, na przykład `(?<level>\w+)` |
| `E_REGEX` | wzorzec nie kompiluje się | popraw wzorzec |
| `E_TYPE` | rekordy wejściowe nie są `line`, `match` ani `content` | najpierw odczytaj tekst za pomocą `lines`, `search` lub `read` |

## Powiązane {#related}

`split`, `search`, `lines`.
