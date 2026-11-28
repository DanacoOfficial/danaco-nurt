---
command: analyze.tools
group: verify
summary: Wyświetla narzędzia analizy wykryte dla projektu i informację, czy są zainstalowane.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# analyze.tools

## Składnia {#synopsis}

```text
analyze.tools <path>... [name: value]...
```

## Opis {#description}

Wyświetla jeden rekord `analyze_tool` dla każdego analizatora i formatera
wybranego przez konfigurację projektu: TypeScript (`tsc`, ESLint, Biome,
Prettier), Python (Ruff, mypy), Go (`go vet`, `gofmt`) i Rust (Clippy,
`rustfmt`). Wykrywanie korzysta z plików konfiguracyjnych i rozszerzeń plików
w zakresie. Program zainstalowany w `node_modules/.bin` jest preferowany
względem programu ze ścieżki wyszukiwania. Wersje są odczytywane za pomocą
`--version`, równolegle.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `list<path>` | brak | Pliki lub katalogi do sprawdzenia. |
| `changed_only` | `bool` | `false` | Tylko języki plików zmienionych w repozytorium. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `analyze_tool` |

Brak wejścia. Wyjście: rekordy `analyze_tool` z polami `tool`, `language`,
`check`, `found`, `version`, `config` i `command`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie narzędzi projektu.

```nurt example
analyze.tools
```

Wyświetlenie narzędzi dla zmienionych plików.

```nurt example
analyze.tools changed_only: true
```

Wyświetlenie narzędzi dla jednego katalogu.

```nurt example
analyze.tools "/home/user/app/services/api"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_EXTERNAL_MISSING` | Podano `changed_only` bez usługi zapytań o repozytorium. | Uruchom z `changed_only: false`. |

## Powiązane {#related}

`analyze.check`, `analyze.format`.
