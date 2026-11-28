---
category: git
title: Git
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cookbook/git.md -->
<!-- Responsibility:   Documents Git. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Git {#git}

## Wyświetlanie stanu repozytorium {#show-the-status-of-a-repository}

Pokaż zmienione i nieśledzone pliki repozytorium.

```nurt example
git.status "/home/user/app"
```

```text expected
/home/user/app
```

```bash
git status --short
```

## Wyświetlanie zmian w poczekalni {#show-the-staged-changes}

Pokaż różnice zmian w poczekalni wraz z ich statystykami.

```nurt example
git.diff "/home/user/app" staged: true stat: true
```

```text expected
files
```

## Wyświetlanie historii pliku {#show-the-history-of-a-file}

Pokaż ostatnie dziesięć rewizji, które zmieniły plik.

```nurt example
git.log "/home/user/app" count: 10 path: ["/home/user/app/src/main.go"]
```

```text expected
commit
```

## Wyświetlanie jednej rewizji {#show-one-commit}

Pokaż zmiany jednej rewizji.

```nurt example
git.show "/home/user/app" "HEAD"
```

```text expected
HEAD
```

## Ustalenie, kto zmienił wiersz {#find-who-changed-a-line}

Pokaż autora każdego wiersza z zakresu pliku.

```nurt example
git.blame "/home/user/app/src/main.go" lines: 10..20
```

```text expected
main.go
```

## Dodanie plików do poczekalni i rewizja {#stage-files-and-commit}

Zatwierdź wybrane pliki z komunikatem.

```nurt example
git.commit "/home/user/app" message: "fix: parser" paths: ["/home/user/app/a.go"]
```

```text expected
fix: parser
```

```bash
git add a.go && git commit -m "fix: parser"
```

## Rewizja ze stopką {#commit-with-a-trailer}

Dodaj wiersz stopki do komunikatu rewizji.

```nurt example
git.commit "/home/user/app" message: "fix: parser" trailers: ["Co-Authored-By: Name <name@example.com>"] staged: true
```

```text expected
fix: parser
```

## Tworzenie gałęzi i przełączenie się na nią {#create-and-switch-to-a-branch}

Utwórz gałąź i przełącz się na nią.

```nurt example
git.switch "/home/user/app" branch: "feature/x" create: true
```

```text expected
feature/x
```

## Zapis punktu kontrolnego przed ryzykowną zmianą {#save-a-checkpoint-before-a-risky-change}

Zapisz stan drzewa roboczego, aby można go było przywrócić.

```nurt example
git.checkpoint "/home/user/app" action: create label: "before refactoring"
```

```text expected
before refactoring
```

## Przywrócenie punktu kontrolnego {#restore-a-checkpoint}

Przywróć drzewo robocze z punktu kontrolnego.

```nurt example
git.checkpoint "/home/user/app" action: restore checkpoint_id: "before refactoring"
```

```text expected
before refactoring
```

## Odłożenie zmian do schowka {#stash-the-changes}

Odłóż zmiany do schowka, łącznie z plikami nieśledzonymi.

```nurt example
git.stash "/home/user/app" action: push include_untracked: true
```

```text expected
stash
```

## Przegląd zmian przed rewizją {#review-the-changes-before-a-commit}

Sprawdź drzewo robocze pod kątem znaczników konfliktów, dużych plików i sekretów.

```nurt example
git.review "/home/user/app" scope: working checks: [conflict_markers, secrets]
```

```text expected
finding
```

## Pobranie zmian z repozytorium zdalnego {#fetch-from-the-remote}

Pobierz gałęzie i usuń odwołania do usuniętych.

```nurt example
git.fetch "/home/user/app" prune: true
```

```text expected
origin
```

## Wypchnięcie gałęzi {#push-a-branch}

Wypchnij gałąź i ustaw jej gałąź nadrzędną (upstream).

```nurt example
git.push "/home/user/app" branch: "feature/x" set_upstream: true
```

```text expected
feature/x
```

## Klonowanie repozytorium {#clone-a-repository}

Sklonuj repozytorium do katalogu.

```nurt example
git.clone "https://example.com/team/app.git" "/home/user/src/app" depth: 1
```

```text expected
/home/user/src/app
```

## Wyświetlanie plików nieśledzonych {#show-the-untracked-files}

Pokaż każdy plik nieśledzony, a nie tylko zwinięte katalogi, wraz z liczbą zmienionych wierszy.

```nurt example
git.status "/home/user/app" untracked: all stat: true
```

```text expected
/home/user/app
```

```bash
git status --short --untracked-files=all
```

## Porównanie dwóch rewizji {#compare-two-revisions}

Pokaż zmiany ścieżki między dwiema rewizjami.

```nurt example
git.diff "/home/user/app" from: "v1.0.0" to: "HEAD" path: ["/home/user/app/src"] stat: true
```

```text expected
files
```

```bash
git diff --stat v1.0.0 HEAD -- src
```

## Wyświetlanie różnic na poziomie słów {#show-a-word-level-diff}

Pokaż zmiany drzewa roboczego na poziomie słów i zignoruj zmiany białych znaków.

```nurt example
git.diff "/home/user/app" word_diff: true ignore_whitespace: true
```

```text expected
hunk
```

## Wyświetlanie rewizji autora {#show-the-commits-of-an-author}

Pokaż rewizje jednego autora od podanej daty.

```nurt example
git.log "/home/user/app" author: "ann" since: "2026-10-01T00:00:00Z" count: 50
```

```text expected
commit
```

```bash
git log --author=ann --since=2026-10-01 -n 50
```

## Wyszukiwanie rewizji, która wprowadziła tekst {#find-the-commit-that-introduced-a-text}

Znajdź rewizje, które dodały lub usunęły podany tekst.

```nurt example
git.log "/home/user/app" pickaxe: "retryLimit" path: ["/home/user/app/src"]
```

```text expected
commit
```

```bash
git log -S retryLimit -- src
```

## Grupowanie rewizji według typu {#group-the-commits-by-type}

Pogrupuj ostatnie 100 rewizji według ich typu konwencjonalnego.

```nurt example
git.log "/home/user/app" count: 100 group_by: type
```

```text expected
group
```

## Wyświetlanie pliku w rewizji {#show-a-file-in-a-revision}

Pokaż zmiany jednego pliku w rewizji wraz z jego statystykami.

```nurt example
git.show "/home/user/app" "HEAD~2" file: "/home/user/app/src/main.go" stat: true
```

```text expected
main.go
```

```bash
git show --stat HEAD~2 -- src/main.go
```

## Wyświetlanie gałęzi {#list-the-branches}

Wyświetl lokalne gałęzie repozytorium.

```nurt example
git.branch "/home/user/app"
```

```text expected
main
```

```bash
git branch
```

## Tworzenie gałęzi z rewizji {#create-a-branch-from-a-revision}

Utwórz gałąź zaczynającą się od znacznika i zachowaj bieżącą gałąź.

```nurt example
git.branch "/home/user/app" create: "hotfix/1.0.1" from: "v1.0.0"
```

```text expected
hotfix/1.0.1
```

```bash
git branch hotfix/1.0.1 v1.0.0
```

## Usunięcie scalonej gałęzi {#delete-a-merged-branch}

Usuń gałąź; repozytorium zachowuje ją w koszu, aby można ją było przywrócić.

```nurt example
git.branch "/home/user/app" delete: "feature/x" trash: true
```

```text expected
feature/x
```

## Dodanie zmian ścieżki do poczekalni {#stage-the-changes-of-a-path}

Dodaj do poczekalni zmiany katalogu.

```nurt example
git.stage "/home/user/app" action: add paths: ["/home/user/app/src"]
```

```text expected
src
```

```bash
git add src
```

## Usunięcie pliku z poczekalni {#unstage-a-file}

Usuń plik z indeksu i zachowaj jego zmiany.

```nurt example
git.stage "/home/user/app" action: unstage paths: ["/home/user/app/a.go"]
```

```text expected
a.go
```

```bash
git restore --staged a.go
```

## Przywrócenie pliku z rewizji {#restore-a-file-from-a-revision}

Przywróć plik do stanu z ostatniej rewizji i najpierw zapisz punkt kontrolny.

```nurt example
git.restore "/home/user/app" paths: ["/home/user/app/a.go"] source: "HEAD" checkpoint: true
```

```text expected
a.go
```

```bash
git restore --source=HEAD a.go
```

## Poprawienie ostatniej rewizji {#amend-the-last-commit}

Zastąp komunikat ostatniej rewizji.

```nurt example
git.commit "/home/user/app" message: "fix: parser error" amend: true
```

```text expected
fix: parser error
```

```bash
git commit --amend -m "fix: parser error"
```

## Tworzenie znacznika z adnotacją {#create-an-annotated-tag}

Utwórz znacznik z adnotacją dla wydania.

```nurt example
git.tag "/home/user/app" name: "v1.2.0" message: "release 1.2.0"
```

```text expected
v1.2.0
```

```bash
git tag -a v1.2.0 -m "release 1.2.0"
```

## Scalenie gałęzi {#merge-a-branch}

Scal gałąź i odrzuć scalenie, które nie jest typu fast-forward.

```nurt example
git.merge "/home/user/app" branch: "feature/x" fast_forward: only
```

```text expected
feature/x
```

## Przerwanie scalenia {#abort-a-merge}

Przerwij scalenie, które zatrzymało się na konflikcie.

```nurt example
git.merge "/home/user/app" action: abort
```

```text expected
effect
```

```bash
git merge --abort
```

## Przebazowanie na gałąź {#rebase-onto-a-branch}

Przebazuj bieżącą gałąź na gałąź główną.

```nurt example
git.rebase "/home/user/app" onto: "main"
```

```text expected
main
```

## Pobranie zmian z przebazowaniem {#pull-with-a-rebase}

Pobierz gałąź nadrzędną i przebazuj na nią lokalne rewizje.

```nurt example
git.pull "/home/user/app" mode: rebase
```

```text expected
origin
```

```bash
git pull --rebase
```

## Zastosowanie schowka i jego usunięcie {#apply-and-drop-a-stash}

Zastosuj najnowszy schowek i usuń go z listy schowków.

```nurt example
git.stash "/home/user/app" action: pop
```

```text expected
stash
```

## Dodanie drzewa roboczego dla gałęzi {#add-a-worktree-for-a-branch}

Wyewidencjonuj gałąź w drugim katalogu.

```nurt example
git.worktree "/home/user/app" action: add path: "/home/user/app-hotfix" branch: "hotfix/1.0.1"
```

```text expected
/home/user/app-hotfix
```

```bash
git worktree add /home/user/app-hotfix hotfix/1.0.1
```

## Wypchnięcie znaczników {#push-the-tags}

Wypchnij gałąź razem ze znacznikami.

```nurt example
git.push "/home/user/app" branch: "main" tags: true
```

```text expected
main
```

## Wypchnięcie z dzierżawą (lease) {#push-with-a-lease}

Zastąp gałąź zdalną tylko wtedy, gdy nie zmieniła się od czasu jej pobrania.

```nurt example
git.push "/home/user/app" branch: "feature/x" force: lease
```

```text expected
feature/x
```

```bash
git push --force-with-lease origin feature/x
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
