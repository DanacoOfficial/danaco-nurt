---
command: git.switch
group: git
summary: Przełącza drzewo robocze na gałąź.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.switch

## Składnia {#synopsis}

```text
git.switch <repo> branch: <text> [name: value]...
```

## Opis {#description}

Aktualizuje drzewo robocze i indeks do drzewa celu oraz ustawia na nie `HEAD`.
Aktualizacja przebiega według reguł dwóch drzew z `git switch`:

- ścieżka równa w bieżącej rewizji i w celu zachowuje swój wpis indeksu i plik
  w drzewie roboczym, więc zmiany lokalne są przenoszone;
- ścieżka, która się różni, jest zapisywana z celu, gdy jej wpis indeksu i plik
  w drzewie roboczym zgadzają się z bieżącą rewizją, i usuwana, gdy brakuje jej
  w celu; katalogi pozostawione puste są usuwane;
- ścieżka, która się różni i ma zmiany lokalne (w indeksie lub w drzewie
  roboczym), powoduje odmowę przełączenia, także gdy zawartość lokalna jest
  równa celowi;
- nieśledzony plik w miejscu, w którym cel ma plik, powoduje odmowę
  przełączenia, chyba że plik jest ignorowany; katalog z nieśledzonymi plikami
  w miejscu, w którym cel ma plik, powoduje odmowę przełączenia.

Wszystkie odmowy są wykrywane, zanim cokolwiek się zmieni, i raportowane
razem. Dziennik `HEAD` otrzymuje wpis `checkout: moving from <old> to <new>`,
także gdy celem jest bieżąca gałąź. Przy `detach` stara i nowa nazwa to nazwa
gałęzi lub pełny identyfikator rewizji, tak jak zapisuje je `git`.

Cel `-` wybiera gałąź wyewidencjonowaną przed bieżącą, odczytaną z dziennika
`HEAD`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `branch` | `text` | wymagany | Gałąź, na którą następuje przełączenie. |
| `create` | `bool` | `false` | Najpierw tworzy gałąź z HEAD. |
| `checkpoint` | `bool` | `false` | Tworzy punkt kontrolny drzewa roboczego przed przełączeniem. |
| `guess` | `bool` | `false` | Tworzy gałąź lokalną z gałęzi repozytorium zdalnego o tej samej nazwie, gdy gałąź lokalna nie istnieje. |
| `start_point` | `text` | brak | Rewizja, od której zaczyna się nowa gałąź; obowiązuje z create. |
| `detach` | `bool` | `false` | Odłącza HEAD na rewizji zamiast przełączać na gałąź. |
| `discard_changes` | `bool` | `false` | Odrzuca zmiany lokalne drzewa roboczego podczas przełączania. |

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
| Warunki odwracalności | Dokładna poprzednia zawartość każdego zmienionego pliku jest przechowywana w dzienniku. |

## Przykłady {#examples}

Przełącz na istniejącą gałąź.

```nurt example
git.switch "/home/user/repo" branch: "feature/login"
```

```text expected
effect git.switch head_after: 4f2a9c1 branch: feature/login
```

Utwórz gałąź na znaczniku i przełącz na nią.

```nurt example
git.switch "/home/user/repo" branch: "fix/1.2" create: true start_point: "v1.2.0"
```

Wyewidencjonuj rewizję bez gałęzi.

```nurt example
git.switch "/home/user/repo" branch: "HEAD~3" detach: true
```

Wróć na poprzednią gałąź.

```nurt example
git.switch "/home/user/repo" branch: "-"
```

Zachowaj bieżące drzewo robocze w punkcie kontrolnym, a następnie przełącz.

```nurt example
git.switch "/home/user/repo" branch: "main" checkpoint: true
```

```text expected
effect git.switch head_after: 9c03e7a branch: main checkpoint_before: cp-4
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_GIT_DIRTY` | Zmiany lokalne, pliki nieśledzone lub nierozwiązane konflikty blokują przełączenie; szczegóły wymieniają ścieżki według rodzaju (`local_changes`, `untracked_overwritten`, `untracked_removed`, `dir_loses_untracked`, `unmerged`). | `git.stash "/home/user/repo" action: push` albo `discard_changes: true` |
| `E_GIT_REF` | Gałąź istnieje (`create`), nazwa jest nieprawidłowa albo nazwa jest rewizją bez `detach` (`expected_branch`). | Pomiń `create` albo dodaj `detach: true` |
| `E_ARG` | `start_point` bez `create` (`start_point_without_create`). | Dodaj `create: true` |
| `E_GIT_REVISION` | Gałąź lub punkt początkowy nie istnieje. | `git.branch "/home/user/repo"`, aby wyświetlić gałęzie |
| `E_GIT_IN_PROGRESS` | Trwa scalanie, przebazowanie, cherry-pick lub revert. | Zakończ lub przerwij operację |
| `E_GIT_LOCKED` | Istnieje `index.lock` lub blokada odwołania. | Zaczekaj na inny proces albo usuń nieaktualną blokadę |
| `E_STALE` | Plik zmienił się między planowaniem a zapisem. | Uruchom polecenie ponownie |

## Powiązane {#related}

`git.restore`, `git.branch`, `git.stash`, `git.checkpoint`, `undo`.
