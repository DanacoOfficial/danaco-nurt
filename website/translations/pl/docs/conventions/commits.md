<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/conventions/commits.md -->
<!-- Responsibility:   Documents Commit conventions. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# Konwencje rewizji {#commit-conventions}

Komunikaty rewizji są zgodne ze specyfikacją [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/),
a każda rewizja jest podpisana (sign-off) zgodnie z Developer Certificate of Origin 1.1
(zob. [CONTRIBUTING.md](../../CONTRIBUTING.md)). Pull requesty są sprawdzane przez
`tools/ci/check-commits.sh`.

Rewizje opublikowane, zanim te kontrole zaczęły ich dotyczyć, są wymienione
z pełnymi 40-znakowymi identyfikatorami w `tools/ci/commits-allow.txt` i
są pomijane przez skrypt, ponieważ opublikowana historia nie jest przepisywana. Nowa
rewizja nigdy nie jest dodawana do tej listy.

## Format {#format}

```text
<type>(<scope>)[!]: <summary>

<body>

<footers>
Signed-off-by: Jane Doe <jane.doe@example.com>
```

### Typ {#type}

| Typ | Zastosowanie |
|---|---|
| `feat` | nowa możliwość widoczna dla użytkowników lub integratorów |
| `fix` | poprawka błędu |
| `perf` | poprawa wydajności bez zmiany zachowania |
| `refactor` | zmiana kodu bez zmiany zachowania |
| `docs` | wyłącznie dokumentacja |
| `test` | wyłącznie testy |
| `build` | konfiguracja budowania, zależności, pakowanie |
| `ci` | ciągła integracja |
| `style` | wyłącznie formatowanie |
| `chore` | utrzymanie, które nie pasuje do innego typu |
| `revert` | cofa wcześniejszą rewizję; treść wskazuje cofniętą rewizję |

### Zakres {#scope}

Zakres to nazwa dotkniętego crate'a bez przedrostka `nurt-` lub obszar
repozytorium:

| Zakres | Zawartość |
|---|---|
| `contracts`, `syntax`, `journal`, `git-ops`, … | crate `nurt-<scope>` w `crates/` |
| `workspace` | główny manifest, plik blokady, plik zestawu narzędzi |
| `deps` | aktualizacje zależności |
| `ci` | przepływy pracy i skrypty ciągłej integracji |
| `spec` | specyfikacja w `spec/` |
| `catalog` | katalog poleceń w `catalog/` |
| `tables` | tabele danych w `tables/` |
| `docs` | dokumentacja w `docs/` |
| `tools` | skrypty w `tools/` |
| `release` | przygotowanie wydania |
| `repo` | pliki repozytorium: licencja, nota, readme, przewodnik współtworzenia |

Zakres można pominąć w zmianie, która dotyczy wielu obszarów.

### Podsumowanie {#summary}

- tryb rozkazujący (`add`, `fix`, `remove`), zaczynający się małą literą;
- bez kropki na końcu;
- co najwyżej 72 znaki dla całego pierwszego wiersza.

### Treść i stopki {#body-and-footers}

Treść wyjaśnia motywację i skutek zmiany, zawinięta na 72
znakach. Stopki mają format trailerów Git:

- `BREAKING CHANGE: <description>` dla zmiany łamiącej zgodność,
  razem z `!` po zakresie;
- `Refs: #<issue>` lub `Closes: #<issue>` dla powiązanych zgłoszeń;
- `Signed-off-by: <name> <e-mail>`, zgodne z autorem rewizji.

## Przykłady {#examples}

```text
feat(journal): record the pre-image of truncated files

Truncation is now recorded before the file is opened for writing, so
that undo restores the original content after an interrupted write.

Closes: #142
Signed-off-by: Jane Doe <jane.doe@example.com>
```

```text
fix(syntax)!: reject duplicate named arguments

BREAKING CHANGE: a stage with the same named argument given twice is now
rejected with an error instead of using the last value.
Signed-off-by: Jane Doe <jane.doe@example.com>
```

```text
build(deps): update regex to 1.13.2

Signed-off-by: Jane Doe <jane.doe@example.com>
```

## Zmiany łamiące zgodność {#breaking-changes}

Zmiana łamiąca zgodność języka, drzewa składni, formatu raportu, katalogu
poleceń lub interfejsu C jest dozwolona wyłącznie w nowej wersji głównej
dotkniętego elementu, zgodnie z definicją w specyfikacji.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
