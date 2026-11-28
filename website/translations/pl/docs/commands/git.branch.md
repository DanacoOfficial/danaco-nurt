---
command: git.branch
group: git
summary: Wyświetla gałęzie albo tworzy je, zmienia ich nazwy, przełącza się na nie lub je usuwa.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.branch

## Składnia {#synopsis}

```text
git.branch <repo> [name: value]...
```

## Opis {#description}

Bez `create`, `rename`, `delete` i `purge` polecenie odczytuje repozytorium i
wyświetla gałęzie lokalne w kolejności `git branch --list` albo gałęzie
śledzące repozytorium zdalne z `remote: true`. Symboliczne odwołania do
repozytorium zdalnego, takie jak `origin/HEAD`, nie są wyświetlane. Każda
gałąź lokalna zawiera swój upstream, liczby rewizji przed nim i za nim,
`upstream_gone`, gdy skonfigurowany upstream już nie istnieje, oraz drzewo
robocze, w którym jest wyewidencjonowana.

`create` zapisuje `refs/heads/<name>` w rewizji `from` oraz wpis reflogu
`branch: Created from <from>`. Gdy `from` wskazuje gałąź śledzącą skonfigurowanego
repozytorium zdalnego, a `branch.autoSetupMerge` nie ma wartości `false`,
konfiguracja otrzymuje `branch.<name>.remote` i `branch.<name>.merge`, tak jak
zapisuje je `git branch`. Z `switch: true` drzewo robocze, indeks i `HEAD`
przechodzą na nową gałąź według reguł `git switch -c`: zmiany lokalne, które
nie powodują konfliktu, są przenoszone, a reflog `HEAD` otrzymuje
`checkout: moving from <old> to <name>`. `checkpoint: true` zapisuje drzewo
robocze i indeks jako punkt kontrolny (`git.checkpoint`) w tej samej operacji,
przed przełączeniem.

`rename` przenosi odwołanie, jego reflog i sekcję `branch.<name>` konfiguracji
oraz przenosi `HEAD` każdego drzewa roboczego wskazującego na gałąź, zapisując
w reflogach `Branch: renamed <old> to <new>`, tak jak `git branch -m`.

`delete` usuwa gałęzie i ich reflogi, także z `packed-refs`. Gałąź jest
scalona, gdy jej rewizja jest osiągalna z jej upstreamu, albo z `HEAD`, gdy nie
ma upstreamu; gałąź, która nie jest scalona, jest usuwana tylko z
`force: true` i jest zgłaszana jako `branch_deleted_unmerged`. Gałąź
wyewidencjonowana w tym lub innym drzewie roboczym nigdy nie jest usuwana. Gdy
klucz konfiguracji `git.trash_refs` ma wartość `true`, wartość każdej usuniętej
gałęzi jest zachowywana jako `refs/nurt/trash/<name>-<seconds>`; `trash: true`
wyświetla zachowane kopie, a `purge` je usuwa, wszystkie za pomocą
`purge: all`. Nazwa w `purge` wybiera kopię według jej odwołania, według
ostatniego składnika odwołania (`<name>-<seconds>`) albo każdą kopię usuniętej
gałęzi według nazwy gałęzi. Nazwa, która jest zarazem ostatnim składnikiem
jednej kopii i nazwą gałęzi innych kopii, taka jak `a-1` dla kopii `a-1` i kopii
`a-1-<seconds>`, jest odrzucana z problemem `ambiguous`; wskaż wtedy odwołania.

Z `check_references` polecenie zgłasza odwołania, które nadal wskazują gałąź o
zmienionej nazwie lub usuniętą: odwołania symboliczne do niej oraz wpisy
`branch.<name>.merge` gałęzi, które ją śledzą z `remote = .`.

`expect_state` przyjmuje `state_token` wcześniejszego skutku `git.branch`.
Token obejmuje `HEAD`, `packed-refs` i gałęzie lokalne; gdy się różni, nic się
nie zmienia, a polecenie kończy się błędem `E_STALE`.

Każdy plik odwołania jest zapisywany zgodnie z protokołem `<file>.lock`
zgodnym z Git, przez dziennik; istniejąca blokada kończy polecenie błędem
`E_GIT_LOCKED`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `remote` | `bool` | `false` | Wyświetla gałęzie śledzące repozytorium zdalne. |
| `create` | `text` | brak | Nazwa gałęzi do utworzenia. |
| `from` | `text` | `"HEAD"` | Punkt początkowy nowej gałęzi. |
| `rename` | `text` | brak | Gałąź, której nazwa ma zostać zmieniona. |
| `new_name` | `text` | brak | Nowa nazwa gałęzi o zmienianej nazwie. |
| `switch` | `bool` | `false` | Przełącza na gałąź po jej utworzeniu. |
| `checkpoint` | `bool` | `false` | Tworzy punkt kontrolny drzewa roboczego przed przełączeniem. |
| `delete` | `text \| list<text>` | brak | Gałęzie do usunięcia. |
| `force` | `bool` | `false` | Usuwa gałęzie, które nie są scalone. |
| `trash` | `bool` | `false` | Wyświetla zachowane kopie usuniętych gałęzi zamiast gałęzi. |
| `purge` | `list<text> \| enum(all)` | brak | Zachowane kopie usuniętych gałęzi do usunięcia. |
| `check_references` | `bool` | `true` | Po usunięciu lub zmianie nazwy zgłasza inne odwołania, które nadal wskazują starą gałąź. |
| `expect_state` | `text` | brak | Token stanu z wcześniejszego odczytu; nic się nie zmienia, gdy stan repozytorium jest inny. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `branch`, `effect`, `git_trash_entry` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |
| Warunki odwracalności | Zmiany gałęzi są odwracalne na podstawie dziennika, który zachowuje poprzednią wartość każdego odwołania. |

| Działanie | Skutek | Odwracalność | Uwaga |
|---|---|---|---|
| `create` | `local` | `reversible` | Obowiązuje, gdy podano create, z switch lub bez. |
| `rename` | `local` | `reversible` | Obowiązuje, gdy podano rename. |
| `delete` | `local` | `reversible` | Obowiązuje, gdy podano delete. |
| `purge` | `local` | `reversible` | Obowiązuje, gdy podano purge. |

## Przykłady {#examples}

Wyświetlenie gałęzi lokalnych wraz z ich upstreamami.

```nurt example
git.branch "/home/user/repo"
```

```text expected
branch name: main current: true upstream: origin/main ahead: 1 behind: 0
```

Utworzenie gałęzi na tagu i wyewidencjonowanie jej z uprzednim zachowaniem punktu kontrolnego.

```nurt example
git.branch "/home/user/repo" create: "fix/1.2" from: "v1.2.0" switch: true checkpoint: true
```

Zmiana nazwy bieżącej gałęzi.

```nurt example
git.branch "/home/user/repo" rename: "master" new_name: "main"
```

Usunięcie gałęzi, która nigdy nie została scalona.

```nurt example
git.branch "/home/user/repo" delete: "spike" force: true
```

Wyświetlenie zachowanych kopii usuniętych gałęzi.

```nurt example
git.branch "/home/user/repo" trash: true
```

```text expected
git_trash_entry name: spike kind: branch ref: refs/nurt/trash/spike-1767225600
```

Usunięcie wszystkich zachowanych kopii usuniętych gałęzi.

```nurt example
git.branch "/home/user/repo" purge: all
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_GIT_REF` | Nazwa jest nieprawidłowa (`invalid_name`), gałąź istnieje (`exists`), nie istnieje (`missing`), jest wyewidencjonowana (`current_branch`, `checked_out`) albo nie jest scalona (`not_merged`). | `git.branch "/home/user/repo" delete: "spike" force: true` |
| `E_GIT_REVISION` | `from` nie wskazuje rewizji. | `git.log "/home/user/repo"` |
| `E_GIT_DIRTY` | Z `switch` zmiany lokalne zostałyby nadpisane. | `git.stash "/home/user/repo" action: push` |
| `E_GIT_LOCKED` | Istnieje blokada odwołania. | Poczekaj na inny proces albo usuń nieaktualną blokadę |
| `E_STALE` | Odwołanie zmieniło się między planowaniem a zapisem albo `expect_state` jest inne. | Odczytaj gałęzie ponownie i powtórz wywołanie |
| `E_ARG` | Parametry wykluczają się wzajemnie (`param_conflict`). | Przekaż jeden parametr zmieniający na wywołanie |
| `E_MISSING_PARAM` | `rename` bez `new_name`. | Dodaj `new_name` |

## Powiązane {#related}

`git.switch`, `git.tag`, `git.checkpoint`, `git.status`, `undo`.
