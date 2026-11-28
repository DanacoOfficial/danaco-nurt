---
command: git.worktree
group: git
summary: Dodaje, wyświetla lub usuwa powiązane drzewa robocze.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.worktree

## Składnia {#synopsis}

```text
git.worktree <repo> action: <enum(add|list|remove)> [name: value]...
```

## Opis {#description}

`add` tworzy powiązane drzewo robocze w katalogu, który nie istnieje albo jest
pusty.
Rejestracja jest zapisywana w `.git/worktrees/<name>/` z plikami `HEAD`,
`ORIG_HEAD`, `commondir`, `gitdir`, `index` i `logs/HEAD`, tak jak zapisuje je
`git worktree add`, a nowy katalog otrzymuje plik `.git` wskazujący na
rejestrację. Dzienniki odwołań `HEAD` i nowej gałęzi są zapisywane zgodnie z
`core.logAllRefUpdates`, a przy wartości `false` nie są zapisywane. `<name>` to
ostatni składnik `path`; gdy nazwa jest zajęta, dopisywana jest najmniejsza
liczba, która czyni ją unikalną (`wt`, `wt1`, `wt2`).

Gałęzią jest `branch`. Bez `branch` i `start_point` jest to ostatni składnik
`path` dokładnie w zapisanej postaci, jak w `git worktree add`; składnik, który
nie jest prawidłową nazwą gałęzi (spacja, `~`, `^`, `:`, `..`, początkowa `.`
lub końcowe `.lock`), kończy się błędem `E_GIT_REF` (`invalid_name`), więc
przekaż `branch`. Bez `branch` wartość `start_point` wskazująca gałąź lokalną
wyewidencjonowuje tę gałąź, a każda inna wartość `start_point` jest
wyewidencjonowywana w stanie odłączonym: `HEAD` zawiera rewizję, a rekord ma
`detached: true`. Istniejąca gałąź jest wyewidencjonowywana i nie może być
wyewidencjonowana w innym drzewie roboczym. Brakująca gałąź jest tworzona na
`start_point` albo na `HEAD`, z komunikatem dziennika
`branch: Created from <start_point>`. Pliki rewizji są zapisywane w nowym
katalogu i odnotowywane w jego indeksie. Sterowniki filtrów zawartości nie są
uruchamiane.

`list` zwraca najpierw główne drzewo robocze, a następnie powiązane drzewa
robocze posortowane według ścieżki, z przyczyną blokady i informacją, czy
katalog nie istnieje (`prunable`). Daje ten sam wynik z dowolnego drzewa
roboczego repozytorium.

`remove` usuwa powiązane drzewo robocze i jego rejestrację. Drzewo robocze ze
zmodyfikowanymi plikami, plikami dodanymi do indeksu lub nieśledzonymi plikami,
które nie są ignorowane, jest usuwane tylko z `force: true`. Pliki ignorowane
nie blokują usunięcia. Gałąź nie jest usuwana.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `action` | `enum(add\|list\|remove)` | wymagany | Operacja na drzewach roboczych. |
| `path` | `path` | brak | Katalog drzewa roboczego do dodania lub usunięcia. |
| `branch` | `text` | brak | Gałąź wyewidencjonowana w nowym drzewie roboczym. |
| `start_point` | `text` | brak | Rewizja, od której zaczyna się nowa gałąź. |
| `force` | `bool` | `false` | Usuwa drzewo robocze, które ma niezatwierdzone zmiany. |
| `checkpoint` | `bool` | `false` | Tworzy punkt kontrolny zmian przed remove. |
| `expect_state` | `text` | brak | Token stanu z wcześniejszego odczytu; nic się nie zmienia, gdy stan repozytorium jest inny. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `git_worktree`, `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |
| Warunki odwracalności | Remove przenosi katalog drzewa roboczego do dziennika przed wyrejestrowaniem go. |

| Akcja | Skutek | Odwracalność | Uwaga |
|---|---|---|---|
| `list` | `read` | `read` |  |

## Przykłady {#examples}

Dodaj drzewo robocze z nową gałęzią.

```nurt example
git.worktree "/home/user/repo" action: add path: "/home/user/repo-hotfix" branch: "hotfix/1.2" start_point: "v1.2.0"
```

```text expected
git_worktree path: /home/user/repo-hotfix branch: hotfix/1.2 main: false
```

Wyświetl drzewa robocze repozytorium.

```nurt example
git.worktree "/home/user/repo" action: list
```

```text expected
git_worktree path: /home/user/repo branch: main main: true
git_worktree path: /home/user/repo-hotfix branch: hotfix/1.2 main: false prunable: false
```

Usuń drzewo robocze, które nadal ma zmiany lokalne.

```nurt example
git.worktree "/home/user/repo" action: remove path: "/home/user/repo-hotfix" force: true
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Brakuje `action` albo, przy `add` i `remove`, `path`. | Przekaż `path` |
| `E_GIT_WORKTREE` | Katalog istnieje i nie jest pusty (`path_exists`), rejestracja już go wskazuje (`already_registered`), ścieżka jest głównym drzewem roboczym (`main_worktree`), nie zarejestrowano tam żadnego drzewa roboczego (`not_found`) albo drzewo robocze jest zablokowane (`locked`). | `git.worktree "/home/user/repo" action: list`; usuń plik `locked` rejestracji |
| `E_GIT_REF` | Gałąź jest wyewidencjonowana w innym drzewie roboczym (`checked_out_elsewhere`), `start_point` podano dla istniejącej gałęzi (`branch_exists`) albo nazwa gałęzi jest nieprawidłowa (`invalid_name`). | Przekaż inną wartość `branch` |
| `E_GIT_REVISION` | `start_point` albo `HEAD` repozytorium bez rewizji nie wskazuje żadnej rewizji. | Przekaż istniejącą rewizję jako `start_point` |
| `E_GIT_DIRTY` | Drzewo robocze do usunięcia ma zmodyfikowane pliki, pliki dodane do indeksu lub nieśledzone (`local_changes`). | `git.worktree "/home/user/repo" action: remove path: "/home/user/repo-hotfix" force: true` |
| `E_GIT_UNSUPPORTED` | Rewizja zawiera niebezpieczną ścieżkę (`unsafe_path`). | Wyewidencjonuj inną rewizję |
| `E_GIT_LOCKED` | Istnieje plik blokady nowej gałęzi. | Zaczekaj na inny proces albo usuń nieaktualny plik blokady |
| `E_STALE` | Gałąź, katalog lub rejestracja pojawiły się między planowaniem a zapisem. | Uruchom polecenie ponownie |

## Powiązane {#related}

`git.switch`, `git.branch`, `git.checkpoint`, `undo`.
