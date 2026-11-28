---
command: git.stash
group: git
summary: Zapisuje niezatwierdzone zmiany w schowku albo wyświetla, pokazuje, stosuje, zdejmuje, usuwa lub czyści wpisy schowka.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.stash

## Składnia {#synopsis}

```text
git.stash <repo> action: <enum(push|pop|apply|list|show|drop|clear)> [name: value]...
```

## Opis {#description}

`push` zapisuje zmiany plików śledzonych, dodane do indeksu i niedodane, jako
nowy wpis schowka i resetuje je do `HEAD`. Wpis ma strukturę używaną przez
`git`: rewizję indeksu (rodzic `HEAD`), przy `include_untracked` rewizję
nieśledzonych plików (bez rodzica) oraz rewizję drzewa roboczego z rodzicami
`HEAD`, rewizją indeksu i rewizją nieśledzonych plików. Jego komunikat to
`WIP on <branch>: <commit> <subject>` albo `On <branch>: <message>` przy
`message`. `refs/stash` wskazuje nowy wpis, a jego dziennik odwołań przechowuje
listę. Przy `paths` zapisywane i resetowane są tylko zmiany pod podanymi
plikami lub katalogami; `exclude` pomija ścieżki pasujące do wzorca (wzorzec bez
`/` pasuje do nazwy pliku). Gdy nie ma zmian, nic nie jest zapisywane.

`apply` scala wpis z drzewem roboczym, przyjmując jako bazę rodzica rewizji
drzewa roboczego tego wpisu; `pop` robi to samo i usuwa wpis, gdy scalenie jest
czyste. Zmiany są stosowane bez dodawania do indeksu, z wyjątkiem plików nowych
we wpisie, które pozostają dodane. Nieśledzone pliki wpisu są zapisywane z
powrotem. Gdy scalenie pozostawia konflikty, pliki zawierają znaczniki
konfliktu oznaczone `Updated upstream` i `Stashed changes`, indeks zawiera
etapy konfliktu, `pop` zachowuje wpis, a polecenie kończy się błędem
`E_GIT_CONFLICT` po zastosowaniu zmiany.

`drop` usuwa jeden wpis, a `clear` usuwa wszystkie przez przepisanie dziennika
odwołań `refs/stash`; odłożone obiekty pozostają w repozytorium. `list` i
`show` tylko odczytują.

Wpisy są nazywane `stash@{N}` lub `N`, gdzie 0 oznacza najnowszy; `apply` i
`show` akceptują także rewizję. Skonfigurowane sterowniki filtrów nie są
uruchamiane.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `action` | `enum(push\|pop\|apply\|list\|show\|drop\|clear)` | wymagany | Operacja na schowku; apply zachowuje wpis, pop go usuwa. |
| `stash` | `text` | najnowszy wpis | Wpis schowka, taki jak stash@{2}. |
| `message` | `text` | brak | Przy push: komunikat nowego wpisu. |
| `include_untracked` | `bool` | `false` | Przy push: zapisuje w schowku także pliki nieśledzone. |
| `paths` | `list<path>` | brak | Przy push: tylko zmiany w tych plikach lub katalogach. |
| `exclude` | `list<glob>` | `[]` | Ścieżki pomijane przy push; raport zlicza pominięte pliki. |
| `expect_state` | `text` | brak | Token stanu z wcześniejszego odczytu; nic się nie zmienia, gdy stan repozytorium jest inny. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect`, `git_stash`, `hunk` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |
| Warunki odwracalności | Dokładna poprzednia zawartość zmienionych plików i odwołania usuniętych wpisów schowka są przechowywane w dzienniku. |

| Akcja | Skutek | Odwracalność | Uwaga |
|---|---|---|---|
| `list` | `read` | `read` |  |
| `show` | `read` | `read` |  |

## Przykłady {#examples}

Zapisz wszystkie zmiany lokalne, w tym nowe pliki nieśledzone.

```nurt example
git.stash "/home/user/repo" action: push message: "parser rewrite" include_untracked: true
```

```text expected
effect git.stash branch: main
```

Wyświetl wpisy.

```nurt example
git.stash "/home/user/repo" action: list
```

```text expected
git_stash index: 0 message: "On main: parser rewrite" includes_untracked: true branch: main
```

Przywróć najnowszy wpis i usuń go.

```nurt example
git.stash "/home/user/repo" action: pop
```

Zapisz tylko zmiany jednego katalogu, pomijając pliki środowiska.

```nurt example
git.stash "/home/user/repo" action: push paths: ["/home/user/repo/src"] exclude: ["*.env"]
```

Usuń drugi najnowszy wpis.

```nurt example
git.stash "/home/user/repo" action: drop stash: "stash@{1}"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `repo` nie znajduje się w repozytorium. | Podaj ścieżkę drzewa roboczego |
| `E_GIT_REF` | Gałąź nie ma jeszcze rewizji (`unborn_branch`), schowek jest pusty (`no_stash`) albo `pop`/`drop` wskazuje rewizję spoza listy (`not_a_stash_reference`). | `git.stash "/home/user/repo" action: list` |
| `E_GIT_REVISION` | Wpis nie istnieje (`out_of_range`) albo nie jest rewizją schowka (`not_a_stash`). | `git.stash "/home/user/repo" action: list` |
| `E_INVALID_VALUE` | Ścieżka z `paths` leży poza repozytorium (`outside_repository`). | Podaj ścieżkę wewnątrz drzewa roboczego |
| `E_NO_MATCH` | Ścieżka z `paths` nie wskazuje żadnego pliku znanego gitowi. | Sprawdź ścieżkę poleceniem `git.status "/home/user/repo"` |
| `E_GIT_IN_PROGRESS` | Indeks zawiera nierozwiązane konflikty (`unmerged_paths`). | Rozwiąż konfliktujące pliki i dodaj je do indeksu |
| `E_GIT_DIRTY` | Zmiany lokalne zostałyby nadpisane (`local_changes`) albo istnieją nieśledzone pliki wpisu (`untracked_overwritten`); ścieżki są w `details.paths`. | `git.stash "/home/user/repo" action: push` albo przenieś pliki |
| `E_GIT_CONFLICT` | Scalenie pozostawiło konflikty (`conflicts`, skutek zastosowany). | Rozwiąż pliki wymienione w `details.conflicts` |
| `E_STALE` | `expect_state` różni się od bieżącego stanu (`state_changed`) albo plik zmienił się między planowaniem a zapisem. | Odczytaj stan ponownie |
| `E_GIT_LOCKED` | Istnieje `index.lock` lub plik blokady odwołania. | Zaczekaj na inny proces albo usuń nieaktualną blokadę |

## Powiązane {#related}

`git.reset`, `git.restore`, `git.status`, `git.checkpoint`, `undo`.
