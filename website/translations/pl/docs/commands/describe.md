---
command: describe
group: text
summary: Wyświetla pola rekordów wejściowych wraz z ich typami i przykładową wartością.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# describe

## Składnia {#synopsis}

```text
any |> describe
```

## Opis {#description}

`describe` wyświetla pola rekordów, które do niego docierają. Zastępuje
badanie obiektu za pomocą `Get-Member` i odpowiada na pytanie, których pól może
użyć następujące po nim polecenie `filter`, `select` lub `sort`.

Wynik zawiera jeden rekord `field_info` na pole:

| Pole | Zawartość |
|---|---|
| `field` | nazwa pola |
| `type` | typ języka wartości; `mixed`, gdy rekordy zawierają w tym polu wartości różnych typów |
| `example` | pierwsza wartość różna od `null` |
| `required` | `true`, gdy każdy rekord wejściowy ma to pole |
| `records` | liczba rekordów wejściowych, które mają to pole |

Pola występują w kolejności pierwszego wystąpienia, więc zachowana jest
kolejność definicji rodzaju rekordu. Pola obliczane na żądanie (`hash`,
`encoding`, `line_ending`, `mime`, `language`, `binary`, `lines`) są wymieniane
tylko wtedy, gdy polecenie tworzące je obliczyło, i wtedy mają `required: false`,
gdy części rekordów ich brakuje.

Puste wejście nie daje żadnych rekordów. Wejście będące pojedynczą wartością
zamiast rekordów kończy się błędem `E_TYPE`. Wynik jest deterministyczny: to
samo wejście daje te same rekordy w tej samej kolejności.

## Parametry {#parameters}

Polecenie nie ma parametrów.

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `any` | `field_info` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `transform` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie pól wpisów katalogu.

```nurt example
list "/home/user/app" |> describe
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app", "type": "dir" },
  { "path": "/home/user/app/a.go", "type": "file", "content_utf8": "package main\n" }
] }
```

```text expected
path
```

Pokazanie wyłącznie pól, których nie ma każdy rekord.

```nurt example
find "/home/user/app" glob: "**/*" |> describe |> filter required == false
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app", "type": "dir" },
  { "path": "/home/user/app/a.go", "type": "file", "content_utf8": "package main\n" }
] }
```

Pokazanie pól dopasowań wyszukiwania wraz z ich typami.

```nurt example
search "main" "/home/user/app" |> describe |> select field type
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app", "type": "dir" },
  { "path": "/home/user/app/a.go", "type": "file", "content_utf8": "package main\n" }
] }
```

```text expected
column
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_TYPE` | wejście jest pojedynczą wartością albo `describe` jest pierwszym etapem i nie ma rekordów do opisania | umieść przed nim źródło, na przykład `list` |

## Powiązane {#related}

`help`, `select`, `filter`, `summarize`.
