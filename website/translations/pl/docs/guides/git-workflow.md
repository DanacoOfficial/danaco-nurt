---
name: git-workflow
description: Praca z repozytoriami git za pomocą wbudowanych poleceń git.*.
commands: [git.status, git.diff, git.log, git.stage, git.commit, git.review, git.checkpoint, git.branch, git.switch, git.merge, git.push, git.fetch, undo]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/git-workflow.md -->
<!-- Responsibility:   Documents git-workflow. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# przepływ pracy z git {#git-workflow}

## Operacje wbudowane {#built-in-operations}

Wszystkie operacje grupy `git` są wbudowane i działają bez zainstalowanego git: status, diff, log, show, blame, branch, stage, commit, tag, fetch, clone, switch, restore, reset, merge, rebase, stash, pull, push, worktree i checkpoint. `fetch`, `clone`, `pull` i `push` działają poza lokalnym systemem plików i są rejestrowane jako nieodwracalne. Niepowodzenia uwierzytelniania są zgłaszane jako `E_GIT_AUTH`, a nieznane hosty SSH jako `E_GIT_HOST_KEY`.

```nurt
git.status "/srv/app"
git.log "/srv/app" count: 5 path: ["/srv/app/a.go"]
```

## Przegląd przed rewizją {#reviewing-before-a-commit}

`git.diff` pokazuje różnice między wersjami, indeksem i drzewem roboczym, a parametry `staged`, `stat` i `symbol` zawężają wynik. `git.review` sprawdza zmiany przed rewizją pod kątem znaczników konfliktu, błędów białych znaków, dużych lub binarnych plików, sekretów, brakujących testów i formatu tematu.

```nurt
git.diff "/srv/app" staged: true stat: true
git.review "/srv/app" scope: staged checks: [conflict_markers, secrets]
```

## Tworzenie rewizji {#committing}

`git.stage` dodaje do indeksu pliki, fragmenty lub wiersze, a `git.commit` zapisuje rewizję z parametrem `message`, `paths` lub `all` oraz opcjonalnie `trailers` i `sign_off`. Hak rewizji, który kończy się niepowodzeniem, zatrzymuje rewizję błędem `E_GIT_HOOK`. Pusta rewizja kończy się błędem `E_GIT_NOTHING_TO_COMMIT`, chyba że ustawiono `allow_empty`.

```nurt
git.commit "/srv/app" message: "fix: parser" paths: ["/srv/app/a.go"]
```

## Punkty kontrolne i cofnięcie {#checkpoints-and-undo}

`git.checkpoint` przechowuje migawkę drzewa roboczego i indeksu poza gałęziami, a `action: restore` do niej wraca. Operacje zmieniające stan lokalny, takie jak commit, switch, merge, rebase, reset i stash, są rejestrowane w dzienniku; `undo` cofa rewizję lub scalenie, przesuwając gałąź z powrotem do jej poprzedniej wersji.

```nurt
git.checkpoint "/srv/app" action: create label: "before-refactoring"
git.checkpoint "/srv/app" action: restore label: "before-refactoring"
```

## Gałęzie i repozytoria zdalne {#branches-and-remotes}

`git.branch` wyświetla, tworzy, zmienia nazwy i usuwa gałęzie, a `git.switch` przełącza drzewo robocze na gałąź. Przełączenie z lokalnymi zmianami, które zostałyby nadpisane, kończy się błędem `E_GIT_DIRTY`. `git.push` z `force: lease` odmawia nadpisania rewizji, które nie zostały pobrane, a wypchnięcie odrzucone przez repozytorium zdalne kończy się błędem `E_GIT_PUSH_REJECTED`.

```nurt
git.switch "/srv/app" branch: "feature/parser" create: true
git.push "/srv/app" remote: "origin" branch: "feature/parser" set_upstream: true
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
