---
command: git.reset
group: git
summary: Przesuwa bieżącą gałąź na rewizję, opcjonalnie resetując indeks i drzewo robocze.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.reset

## Składnia {#synopsis}

```text
git.reset <repo> revision: <text> [name: value]...
```

## Opis {#description}

Przesuwa bieżącą gałąź albo odłączony `HEAD` na rewizję wskazaną przez
`revision`. Tryb decyduje, co jeszcze jest resetowane:

- `soft` zmienia tylko odwołanie. Jest odrzucany, gdy trwa scalanie
  lub indeks zawiera konflikty.
- `mixed` zastępuje także indeks drzewem rewizji. Wpisy indeksu, których obiekt
  i tryb się nie zmieniają, zachowują dane o stanie pliku; drzewo robocze nie
  jest zmieniane.
- `hard` doprowadza także drzewo robocze do stanu rewizji. Lokalne zmiany plików
  śledzonych są odrzucane, a nieśledzony plik stojący na drodze pliku
  śledzonego z rewizji jest nadpisywany. Pozostałe pliki nieśledzone zostają.

Poprzednia rewizja `HEAD` jest zapisywana w `ORIG_HEAD`. Dziennik odwołań
gałęzi otrzymuje wpis `reset: moving to <revision>`, gdy gałąź się przesuwa;
dziennik odwołań `HEAD` otrzymuje ten sam wpis. Pliki stanu trwającego
scalania, cherry-pick lub revert (`MERGE_HEAD`, `MERGE_MSG`, `MERGE_MODE`,
`MERGE_RR`, `AUTO_MERGE`, `SQUASH_MSG`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`)
są usuwane. Skonfigurowane sterowniki filtrów nie są uruchamiane; stosowane są
wbudowane konwersje końców wierszy, `ident` i kodowania drzewa roboczego.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `revision` | `text` | wymagany | Rewizja, na którą następuje przesunięcie. |
| `mode` | `enum(soft\|mixed\|hard)` | `mixed` | Zachowuje indeks i drzewo robocze, resetuje indeks albo resetuje oba. |

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
| Warunki odwracalności | Poprzednie odwołanie i dokładna poprzednia zawartość każdego zmienionego pliku są przechowywane w dzienniku. |

## Przykłady {#examples}

Cofnij ostatnią rewizję i zachowaj jej zmiany w indeksie.

```nurt example
git.reset "/home/user/repo" revision: "HEAD~1" mode: soft
```

```text expected
effect git.reset branch: main notable: history_rewritten
```

Usuń wszystko z indeksu, zachowując drzewo robocze.

```nurt example
git.reset "/home/user/repo" revision: "HEAD"
```

Odrzuć wszystkie lokalne zmiany plików śledzonych i wróć do gałęzi zdalnej.

```nurt example
git.reset "/home/user/repo" revision: "origin/main" mode: hard
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `repo` nie znajduje się w repozytorium. | Podaj ścieżkę drzewa roboczego |
| `E_GIT_UNSUPPORTED` | Repozytorium jest typu bare (`bare_repository`) albo indeks jest rzadki (`sparse_index`). | Użyj drzewa roboczego z pełnym indeksem |
| `E_GIT_REVISION` | Rewizja nie istnieje (`unknown`) albo nie jest rewizją (`not_a_commit`). | Sprawdź nazwę poleceniem `git.log "/home/user/repo"` |
| `E_GIT_IN_PROGRESS` | `soft` podczas scalania (`merge_in_progress`). | `git.reset "/home/user/repo" revision: "HEAD" mode: mixed` |
| `E_GIT_LOCKED` | Istnieje `index.lock` lub plik blokady odwołania. | Zaczekaj na inny proces albo usuń nieaktualną blokadę |
| `E_UNKNOWN_VALUE` | `mode` nie jest jednym z trybów. | Użyj `soft`, `mixed` lub `hard` |
| `E_STALE` | Plik zmienił się między planowaniem a zapisem. | Uruchom polecenie ponownie |

## Powiązane {#related}

`git.restore`, `git.switch`, `git.stash`, `git.log`, `undo`.
