---
command: git.status
group: git
summary: Pokazuje stan drzewa roboczego i indeksu jednego lub wielu repozytoriów.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.status

## Składnia {#synopsis}

```text
git.status <repo>... [name: value]...
```

## Opis {#description}

Wyświetla ścieżki każdego repozytorium, których indeks różni się od `HEAD`,
których plik w drzewie roboczym różni się od indeksu, które mają etapy konfliktu
oraz które są nieśledzone lub ignorowane. Wpisy i ich stany odpowiadają
`git status --porcelain=v2`.

Kolejność w obrębie jednego repozytorium jest stała: ścieżki śledzone i ścieżki
z konfliktami w kolejności bajtów ścieżki, następnie ścieżki nieśledzone, a
potem ignorowane. Repozytoria występują w kolejności `repo`.
Ścieżka repozytorium, która nie jest prawidłowym Unicode, jest pokazywana z
U+FFFD w `path` i `renamed_from`, a jej bajty znajdują się w `raw_path` i
`raw_renamed_from`.

Indeks jest porównywany z `HEAD` z wykrywaniem zmian nazw i kopii zgodnie z
`status.renames`, którego wartością domyślną jest wartość `diff.renames`
(włączone); zmiana nazwy wymaga podobieństwa co najmniej 50 procent. Drzewo
robocze jest porównywane z indeksem bez wykrywania zmian nazw. Ścieżki z
etapami konfliktu niosą rodzaj konfliktu wynikający z etapów obecnych w
indeksie.

Przy `untracked: normal` katalog, który nie zawiera żadnego pliku śledzonego,
jest jednym wpisem, którego ścieżka kończy się na `/`; `all` wyświetla każdy
plik nieśledzony, a `no` żaden. `ignored` dodaje ścieżki ignorowane,
zwijane w ten sam sposób. Puste katalogi nie są wyświetlane.

Podmoduł jest raportowany jako zmodyfikowany, gdy jego wyewidencjonowana
rewizja różni się od zapisanej, gdy ma zmodyfikowane pliki śledzone albo gdy ma
pliki nieśledzone; pole `submodule` wskazuje, które z tych zdarzeń zaszło.
Wpisy indeksu spoza rzadkiego wyewidencjonowania nie są porównywane z drzewem
roboczym.

Sterowniki filtrów (`filter.<name>.clean`) nie są uruchamiane; pliki są
porównywane po wbudowanych konwersjach (`core.autocrlf`, `eol`, `text`,
`ident`, `working-tree-encoding`), a raport zawiera uwagę `filter_not_applied`.
`core.fsmonitor` nie jest używany i jest odnotowany jako `fsmonitor_ignored`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `list<path>` | wymagany | Repozytoria lub ścieżki wewnątrz nich. |
| `untracked` | `enum(normal\|all\|no)` | `normal` | Pliki nieśledzone: katalogi zwinięte, każdy plik albo żadne. |
| `ignored` | `bool` | `false` | Uwzględnia pliki ignorowane. |
| `limit` | `int` | `200` | Maksymalna liczba wpisów na repozytorium. |
| `stat` | `bool` | `false` | Uwzględnia liczby dodanych i usuniętych wierszy. |
| `exclude` | `list<glob>` | `[]` | Ścieżki do pominięcia; raport zlicza pominięte pliki. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `git_status` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Pokaż zmiany jednego repozytorium.

```nurt example
git.status "/home/user/repo"
```

```text expected
git_status path: src/main.rs index: unmodified worktree: modified kind: tracked
git_status path: notes/ index: untracked worktree: untracked kind: untracked
```

Wyświetl każdy plik nieśledzony razem z plikami ignorowanymi.

```nurt example
git.status "/home/user/repo" untracked: all ignored: true
```

Zlicz zmienione wiersze w dwóch repozytoriach, pomijając pliki blokad.

```nurt example
git.status ["/home/user/app", "/home/user/lib"] stat: true exclude: ["*.lock"]
```

Wyświetl tylko ścieżki z konfliktami po scaleniu.

```nurt example
git.status "/home/user/repo" untracked: no |> filter kind == "conflict"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | Ścieżka nie znajduje się w repozytorium albo repozytorium jest typu bare (problem `bare_repository`). | Podaj ścieżkę drzewa roboczego |
| `E_INVALID_VALUE` | `untracked` nie jest `normal`, `all` ani `no` albo `limit` jest poza zakresem od 1 do 5000. | Użyj wartości z listy |
| `E_GIT_UNSUPPORTED` | Repozytorium używa formatu, który nie jest zaimplementowany, takiego jak `reftable`. | Użyj repozytorium z silnikiem odwołań opartym na plikach |
| `E_STAGE_FAILED` | Nie można odczytać indeksu, obiektu lub pliku (problem `read_failed`). | Przywróć brakujące obiekty repozytorium |

## Powiązane {#related}

`git.diff`, `git.stage`, `git.commit`, `git.review`, `git.checkpoint`, `undo`.
