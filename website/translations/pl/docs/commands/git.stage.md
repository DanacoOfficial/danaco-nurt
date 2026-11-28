---
command: git.stage
group: git
summary: Wyświetla, dodaje do indeksu lub usuwa z indeksu zmiany, z dokładnością do wybranych fragmentów i wierszy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.stage

## Składnia {#synopsis}

```text
git.stage <repo> action: <enum(list|add|unstage)> [name: value]...
```

## Opis {#description}

`action: list` raportuje zmiany drzewa roboczego względem indeksu albo, przy
`staged: true`, indeksu względem `HEAD`, jako rekordy `hunk`. Każdy fragment ma
identyfikator `<path>#<number>:<fingerprint>`, który akceptują `add` i
`unstage`; odcisk powoduje, że późniejsze wywołanie kończy się błędem `E_STALE`,
gdy fragment w międzyczasie się zmienił.

`action: add` kopiuje stan drzewa roboczego z wybranego zakresu do indeksu, tak
jak robi to `git add`: zmienione i nowe pliki są dodawane do indeksu, pliki
usunięte z drzewa roboczego są usuwane z indeksu, a plik z konfliktem dodany do
indeksu jest uznawany za rozwiązany, przy czym jego etapy są zachowywane jako
dane resolve-undo. Katalog, a także `.` dla całego drzewa roboczego, oznacza
pliki pod nim: wpisy indeksu i nieśledzone pliki, które nie są ignorowane.
Pliki pasujące do wzorca z `exclude` są pomijane i zliczane. Ignorowany plik
wskazany jawnie w `paths` jest dodawany do indeksu, czego `git add` odmawia bez
`-f`; raport zawiera uwagę `ignored`, która wymienia takie pliki.

`action: unstage` kopiuje stan `HEAD` z wybranego zakresu do indeksu, tak jak
robi to `git restore --staged`: wpis wraca do zawartości i trybu z `HEAD`, wpis
nieobecny w `HEAD` jest usuwany, a drzewo robocze się nie zmienia.

`hunks` wybiera części plików. `<path>#<number>` wybiera cały fragment, a
`<path>#<number>@<rows>` wybrane jego wiersze, gdzie wiersze to liczby i
zakresy, takie jak `3-5,8`, liczone po wierszach fragmentu. Dla `add`
niewybrany dodany wiersz jest odrzucany, a niewybrany usunięty wiersz
pozostaje; dla `unstage` wybór jest usuwany z zawartości indeksu. Wynik jest
równy `git apply --cached` z łatką zredukowaną do wyboru. Nowy plik można dodać
do indeksu częściowo; pliki bez końcowego znaku nowego wiersza i z końcami
wierszy CRLF zachowują swoje bajty.

Indeks jest zapisywany pod `index.lock` przez dziennik; nowe obiekty blob są
dodawane do bazy obiektów. Wpisy indeksu zapisane przez polecenie nie niosą
stanu pliku, tak jak wpisy zapisane przez `git apply --cached`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `action` | `enum(list\|add\|unstage)` | wymagany | Wyświetla zmiany jako fragmenty, dodaje je do indeksu albo usuwa z indeksu. |
| `paths` | `list<path>` | brak | Pliki lub katalogi, na których wykonywana jest operacja. |
| `hunks` | `list<text>` | brak | Fragmenty według identyfikatora, takie jak file#2 lub file#2@3-5,8 dla wybranych wierszy. |
| `staged` | `bool` | brak | Przy list: pokazuje zmiany w indeksie zamiast zmian poza indeksem. |
| `exclude` | `list<glob>` | `[]` | Ścieżki pomijane przy add; raport zlicza pominięte pliki. |
| `intent_to_add` | `bool` | `false` | Przy add: zapisuje ścieżki w indeksie bez ich zawartości. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect`, `hunk` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

| Akcja | Skutek | Odwracalność | Uwaga |
|---|---|---|---|
| `list` | `read` | `read` |  |

## Przykłady {#examples}

Wyświetl fragmenty jednego pliku niedodane do indeksu.

```nurt example
git.stage "/home/user/repo" action: list paths: ["/home/user/repo/src/main.rs"]
```

```text expected
hunk path: src/main.rs id: src/main.rs#1:9c0d2e old_start: 12 new_start: 12
```

Dodaj do indeksu drugi fragment oraz wiersze od 3 do 5 pierwszego fragmentu.

```nurt example
git.stage "/home/user/repo" action: add hunks: ["src/main.rs#2", "src/main.rs#1@3-5"]
```

Dodaj do indeksu katalog bez plików dziennika.

```nurt example
git.stage "/home/user/repo" action: add paths: ["/home/user/repo/src"] exclude: ["*.log"]
```

Usuń z indeksu jeden fragment pliku dodanego do indeksu.

```nurt example
git.stage "/home/user/repo" action: unstage hunks: ["README.md#1"]
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | `add` lub `unstage` bez `paths` i `hunks` (`nothing_selected`). | `git.stage "/home/user/repo" action: list` |
| `E_ARG` | Ścieżka leży poza drzewem roboczym albo wewnątrz `.git` (`invalid_path`) albo identyfikator lub wybór wierszy jest nieprawidłowy (`invalid_id`, `invalid_rows`). | Użyj ścieżki względem katalogu głównego drzewa roboczego |
| `E_NOT_FOUND` | Plik nie istnieje ani w drzewie roboczym, ani w indeksie, ani w `HEAD` (`pathspec_no_match`) albo fragment nie istnieje (`not_found`). | Wyświetl fragmenty ponownie |
| `E_STALE` | Odcisk fragmentu już nie pasuje (`changed`) albo indeks zmienił się między planowaniem a zapisem. | Wyświetl fragmenty ponownie |
| `E_GIT_LOCKED` | Istnieje `index.lock`. | Zaczekaj na inny proces albo usuń nieaktualną blokadę |
| `E_GIT_UNSUPPORTED` | Repozytorium nie ma drzewa roboczego (`bare`). | Uruchom polecenie w drzewie roboczym |

## Powiązane {#related}

`git.status`, `git.diff`, `git.commit`, `git.restore`, `undo`.
