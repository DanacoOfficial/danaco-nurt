---
command: git.rebase
group: git
summary: Odtwarza rewizje bieżącej gałęzi na innej rewizji albo kontynuuje, pomija lub przerywa przebazowanie.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.rebase

## Składnia {#synopsis}

```text
git.rebase <repo> [name: value]...
```

## Opis {#description}

Przy `action: start` (wartość domyślna) rewizje z zakresu `merge-base(onto, HEAD)..HEAD`
są odtwarzane na rewizji wskazanej przez `onto`, tak jak robi to `git rebase <onto>`
z silnikiem `merge`. Rewizje scalające są pomijane, podobnie jak rewizje, których
identyfikator łatki jest równy identyfikatorowi rewizji już obecnej w `onto`. Rewizje
są odtwarzane od najstarszej: każda jest scalana z wynikiem poprzedniej
trójstronnym scalaniem z `git.merge`, a nowa rewizja zachowuje autora, datę
autorstwa i komunikat rewizji oryginalnej; osoba zatwierdzająca i data rewizji
pochodzą ze środowiska sesji. Rewizja, której zmiana jest już obecna w nowej
bazie, jest odrzucana; rewizja, która od początku była pusta, jest zachowywana.
Gdy `HEAD` zawiera już `onto` na linii historii bez scaleń, nic się nie zmienia
i stan to `up_to_date`; gdy bieżąca gałąź nie ma własnych rewizji, zostaje
przesunięta na `onto`, a stan to `fast_forward`.

Odtwarzanie przebiega w bazie obiektów. Drzewo robocze i indeks są zapisywane
jednorazowo, ze stanu sprzed wywołania do stanu końcowego, a gałąź jest
przesuwana na ostatnią odtworzoną rewizję.

Gdy rewizji nie można odtworzyć bez konfliktów, przebazowanie zatrzymuje się:
`HEAD` jest odłączony na ostatniej odtworzonej rewizji, drzewo robocze zawiera
scalone pliki ze znacznikami konfliktu oznaczonymi `HEAD` i `<short> (<subject>)`,
a indeks zawiera etapy 1, 2 i 3 każdej konfliktującej ścieżki. Katalog stanu
`rebase-merge/` oraz pliki `REBASE_HEAD`, `MERGE_MSG`, `AUTO_MERGE` i `ORIG_HEAD`
są zapisywane w formacie Git 2.53, dzięki czemu Git także może kontynuować
przebazowanie, a przebazowanie zatrzymane przez Git można kontynuować tutaj.
Wywołanie kończy się błędem `E_GIT_CONFLICT` z `effect_applied: true`; szczegóły
zawierają konflikty i krok (`done`, `total`). Rewizja, której wynik nadpisałby
nieśledzony plik, zatrzymuje przebazowanie w ten sam sposób z błędem `E_GIT_DIRTY`
(`untracked_overwritten`); rewizja pozostaje pierwsza na liście zadań.

`action: continue` zatwierdza rozwiązany indeks z autorem i komunikatem
zatrzymanej rewizji, bez sekcji `# Conflicts:`, i odtwarza pozostałe rewizje.
Każdy konflikt musi być rozwiązany i dodany do indeksu, a drzewo robocze nie może
mieć niedodanych zmian. Gdy indeks jest równy `HEAD`, rewizja nie powstaje.

`action: skip` odrzuca zatrzymaną rewizję, usuwa jej zmiany z indeksu i drzewa
roboczego oraz odtwarza pozostałe rewizje.

`action: abort` przywraca drzewo robocze, indeks i `HEAD` do stanu sprzed
przebazowania i usuwa pliki stanu.

Każda kontrola, która może odmówić przebazowania, jest wykonywana przed pierwszą
zmianą: operacja w toku, nierozwiązane ścieżki, zmiany dodane do indeksu i
niedodane oraz nieśledzone pliki stojące na drodze `onto`. Haki nie są
uruchamiane (`hooks_skipped`), a programy sterowników scalania nie są
uruchamiane podczas przebazowania.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `onto` | `text` | brak | Rewizja, na którą następuje przebazowanie; wymagana, gdy action to start. |
| `action` | `enum(start\|continue\|skip\|abort)` | `start` | Rozpoczyna przebazowanie, kontynuuje je, pomija bieżącą rewizję albo je przerywa. |

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
| Warunki odwracalności | Poprzednie odwołania i dokładna poprzednia zawartość każdego zmienionego pliku są przechowywane w dzienniku. |

## Przykłady {#examples}

Odtwórz rewizje bieżącej gałęzi na `main`.

```nurt example
git.rebase "/home/user/project" onto: "main"
```

```text expected
effect.git state: completed branch: topic
```

Przebazuj na gałąź śledzącą repozytorium zdalne po pobraniu zmian.

```nurt example
git.fetch "/home/user/project"
git.rebase "/home/user/project" onto: "origin/main"
```

Kontynuuj przebazowanie po rozwiązaniu konfliktów i dodaniu plików do indeksu.

```nurt example
git.stage "/home/user/project" action: add paths: ["/home/user/project/src/main.rs"]
git.rebase "/home/user/project" action: continue
```

Odrzuć rewizję, na której zatrzymało się przebazowanie.

```nurt example
git.rebase "/home/user/project" action: skip
```

Przerwij przebazowanie i wróć do gałęzi w poprzednim stanie.

```nurt example
git.rebase "/home/user/project" action: abort
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_GIT_CONFLICT` | Przebazowanie zatrzymało się z konfliktami (`effect_applied: true`) albo indeks zawiera nierozwiązane ścieżki (`unmerged_paths`). | Rozwiąż pliki, dodaj je do indeksu i uruchom `action: continue` albo uruchom `action: skip` lub `action: abort`. |
| `E_GIT_DIRTY` | Zmiany dodane do indeksu (`staged_changes`), zmiany niedodane (`unstaged_changes`) lub nieśledzone pliki stojące na drodze (`untracked_overwritten`); przy `effect_applied: true` przebazowanie zatrzymało się na rewizji. | Zatwierdź zmiany lub odłóż je w schowku: `git.stash "<repo>" action: push`; przenieś nieśledzone pliki. |
| `E_GIT_IN_PROGRESS` | W toku jest inna operacja albo żadne przebazowanie nie jest zatrzymane dla `continue`, `skip` i `abort` (`no_rebase_in_progress`). | Zakończ lub przerwij operację w toku. |
| `E_GIT_REVISION` | `onto` nie wskazuje żadnej rewizji. | Podaj istniejącą gałąź, znacznik lub rewizję. |
| `E_GIT_REF` | `HEAD` nie ma rewizji (`unborn_branch`). | Utwórz pierwszą rewizję. |
| `E_GIT_SIGN` | Ustawiono `commit.gpgSign`; odtwarzanych rewizji nie można podpisać. | Ustaw `commit.gpgSign` na `false` dla repozytorium. |
| `E_GIT_UNSUPPORTED` | Ustawiono `rebase.updateRefs`, ustawiono `rebase.autoStash` przy niezatwierdzonych zmianach, lista zadań zawiera polecenia interaktywne albo zatrzymane przebazowanie używa silnika `apply`. | Zakończ przebazowanie w Git albo zmień ustawienie. |
| `E_GIT_LOCKED` | Istnieje `index.lock` lub blokada odwołania. | Zaczekaj na inny proces Git albo usuń plik blokady. |
| `E_MISSING_PARAM` | `action: start` bez `onto`. | Podaj `onto: "<revision>"`. |

## Powiązane {#related}

`git.merge`, `git.pull`, `git.status`, `git.stage`, `git.stash`,
`git.log`, `undo`.
