---
command: git.merge
group: git
summary: Scala gałąź z bieżącą gałęzią albo kontynuuje lub przerywa scalanie.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.merge

## Składnia {#synopsis}

```text
git.merge <repo> [name: value]...
```

## Opis {#description}

Z `action: start` (wartość domyślna) rewizja wskazana przez `branch` jest
scalana z bieżącą gałęzią. Gdy bieżąca rewizja jest przodkiem scalanej rewizji,
gałąź jest przesuwana w trybie fast-forward: drzewo robocze, indeks i gałąź
przechodzą na scalaną rewizję, a żadna rewizja nie jest tworzona. Gdy scalana
rewizja jest już zawarta w bieżącej gałęzi, nic się nie zmienia, a stan to
`up_to_date`. W przeciwnym razie obliczane jest scalanie trójstronne względem
najlepszego wspólnego przodka; kilku najlepszych przodków jest najpierw
scalanych w wirtualnego przodka. Tekst jest scalany wiersz po wierszu
algorytmem histogram, zmiany nazw są śledzone zgodnie z `merge.renames` i
`merge.renameLimit`, a atrybuty `merge` i `conflict-marker-size` są
respektowane.

Czyste scalanie trójstronne tworzy rewizję scalającą, której rodzicami są
bieżąca rewizja i scalana rewizja. Jej komunikat to `Merge branch '<name>'`,
`Merge remote-tracking branch '<name>'`, `Merge tag '<name>'` albo
`Merge commit '<revision>'`, a po nim ` into <branch>`, chyba że bieżąca gałąź
pasuje do `merge.suppressDest` (domyślnie `main` i `master`).

Gdy scalanie ma konflikty albo gdy `no_commit` ma wartość true, scalanie
zatrzymuje się: drzewo robocze zawiera scalone pliki ze znacznikami
konfliktów, indeks zawiera etapy 1, 2 i 3 każdej ścieżki w konflikcie, a pliki
stanu `MERGE_HEAD`, `MERGE_MSG`, `MERGE_MODE`, `AUTO_MERGE` i `ORIG_HEAD` są
zapisywane w formacie Git, więc Git również może zakończyć scalanie.
Wywołanie kończy się wówczas błędem `E_GIT_CONFLICT` i `effect_applied: true`;
rekord `effect.git` wymienia konflikty.

`action: continue` kończy zatrzymane scalanie rewizją z indeksu. Wymaga, aby
każdy konflikt został rozwiązany i dodany do poczekalni. Komunikatem jest
przygotowany `MERGE_MSG` bez wierszy komentarza albo `message`, gdy podano.

`action: abort` przywraca indeks i drzewo robocze do `HEAD` dla każdej
ścieżki, którą zmieniło scalanie, i usuwa pliki stanu scalania. Zmiany lokalne
ścieżek, których scalanie nie dotknęło, są zachowywane.

Każde sprawdzenie, które może odrzucić scalanie, jest wykonywane przed pierwszą
zmianą: trwająca operacja, nierozwiązane ścieżki, zmiany w poczekalni (dla
scalania trójstronnego), zmiany lokalne i pliki nieśledzone stojące na drodze
wyniku. Skonfigurowane sterowniki scalania (`merge.<name>.driver`) nie są
uruchamiane. Wbudowany sterownik tekstu scala plik, a ścieżka jest zgłaszana
jako `filter_not_applied`.

Zmiany nazw katalogów są stosowane tak jak z `merge.directoryRenames=true`:
ścieżka dodana po jednej stronie w katalogu, którego nazwę zmieniła druga
strona, jest przenoszona do katalogu o nowej nazwie, a scalanie pozostaje
czyste. Git z domyślnym `merge.directoryRenames=conflict` wykonuje to samo
przeniesienie i dodatkowo zgłasza konflikt `file location`, pozostawiając
przeniesioną ścieżkę na etapie 3 lub 2. Ustawienie `merge.directoryRenames` nie
jest odczytywane.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `branch` | `text` | brak | Gałąź do scalenia; wymagana, gdy action ma wartość start. |
| `action` | `enum(start\|continue\|abort)` | `start` | Rozpoczyna scalanie, kontynuuje je po rozwiązaniu konfliktów albo je przerywa. |
| `fast_forward` | `enum(auto\|only\|never)` | `auto` | Fast-forward, gdy to możliwe, tylko fast-forward albo zawsze tworzenie rewizji scalającej. |
| `message` | `text` | brak | Komunikat rewizji scalającej. |
| `no_commit` | `bool` | `false` | Zatrzymuje przed utworzeniem rewizji scalającej. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect`, `git_conflict` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |
| Warunki odwracalności | Poprzednie odwołania i dokładna poprzednia zawartość każdego zmienionego pliku są zachowane w dzienniku. |

## Przykłady {#examples}

Scalenie gałęzi `topic` z bieżącą gałęzią.

```nurt example
git.merge "/home/user/project" branch: "topic"
```

```text expected
effect.git state: completed branch: main
```

Utworzenie rewizji scalającej z własnym komunikatem, nawet gdy gałąź dałoby się
przesunąć w trybie fast-forward.

```nurt example
git.merge "/home/user/project" branch: "release/1.4" fast_forward: never message: "Merge release 1.4"
```

Zakończenie scalania po rozwiązaniu konfliktów i dodaniu ich do poczekalni.

```nurt example
git.stage "/home/user/project" action: add paths: ["/home/user/project/src/main.rs"]
git.merge "/home/user/project" action: continue
```

Przerwanie scalania, które zatrzymało się z konfliktami.

```nurt example
git.merge "/home/user/project" action: abort
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_GIT_CONFLICT` | Scalanie zatrzymało się z konfliktami (`effect_applied: true`) albo indeks ma nierozwiązane ścieżki (`unmerged_paths`). | Rozwiąż pliki, dodaj je do poczekalni i uruchom `action: continue` albo uruchom `action: abort`. |
| `E_GIT_DIRTY` | Na drodze stoją zmiany w poczekalni (`staged_changes`), zmiany lokalne (`local_changes`) lub pliki nieśledzone. | Zapisz zmiany w rewizji lub odłóż je do schowka: `git.stash "<repo>" action: push`. |
| `E_GIT_IN_PROGRESS` | Trwa inna operacja albo żadne scalanie nie jest zatrzymane dla `continue` i `abort` (`no_merge_in_progress`). | Zakończ lub przerwij trwającą operację. |
| `E_GIT_NON_FAST_FORWARD` | `fast_forward: only`, a historie się rozeszły (`not_possible`), albo historie są niepowiązane (`unrelated_histories`). | Scal z `fast_forward: auto` albo wykonaj rebase na gałąź. |
| `E_GIT_REVISION` | `branch` nie wskazuje rewizji. | Podaj istniejącą gałąź, tag lub rewizję. |
| `E_GIT_LOCKED` | Istnieje `index.lock` lub blokada odwołania. | Poczekaj na inny proces Git albo usuń plik blokady. |
| `E_MISSING_PARAM` | `action: start` bez `branch`. | Podaj `branch: "<name>"`. |

## Powiązane {#related}

`git.status`, `git.stage`, `git.commit`, `git.rebase`, `git.pull`,
`git.stash`, `undo`.
