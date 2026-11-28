---
command: git.restore
group: git
summary: Przywraca pliki lub wybrane fragmenty zmian w drzewie roboczym albo indeksie z rewizji.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.restore

## Składnia {#synopsis}

```text
git.restore <repo> paths: <list<path>> [name: value]...
```

## Opis {#description}

Kopiuje ścieżki wybrane przez `paths` ze źródła do drzewa roboczego, indeksu
albo do obu. Ścieżki używają składni pathspec programu `git` względem katalogu
głównego repozytorium: katalog wybiera wszystko, co leży pod nim, a `*.txt` i
`:(glob)**/x` są wzorcami. Pathspec, który nie pasuje do żadnej ścieżki źródła
ani indeksu, jest błędem i nic się nie zmienia.

Źródłem jest rewizja `source` (rewizja, znacznik lub drzewo). Bez niej drzewo
robocze jest przywracane z indeksu, a indeks (z `staged`) z `HEAD`; przy
jednoczesnym `staged` i `worktree` oba pochodzą z `HEAD`.

Przywracanie nie nakłada zmian: wybrana ścieżka obecna w indeksie, a nieobecna
w źródle, jest usuwana z przywracanej strony. Drzewo robocze jest zapisywane
tylko tam, gdzie jego zawartość różni się od źródła. Ścieżki z nierozwiązanymi
konfliktami nie można przywrócić z indeksu do drzewa roboczego; przywróć ją
zamiast tego z rewizji przy `staged: true`.

Przy `hunks` cofane są tylko wskazane fragmenty zmian, tak jak robi to
`git restore --patch`. Identyfikatory mają postać jak w `git.stage`
(`path#number`, opcjonalnie `:fingerprint` i `@rows`). Przy `worktree` fragmenty
to te między indeksem a plikiem drzewa roboczego; przy `staged: true worktree: false`
to te między `HEAD` a indeksem, a nowy plik, którego jedyny fragment zostaje
cofnięty, opuszcza indeks. Niewybrane fragmenty pozostają bez zmian. Każdy
fragment musi leżeć w pliku znajdującym się w jednej ze ścieżek `paths` lub pod
nią, porównywanych dosłownie. `source` i przywracanie obu stron nie mogą być
łączone z `hunks`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `paths` | `list<path>` | wymagany | Pliki lub katalogi do przywrócenia. |
| `source` | `text` | indeks albo HEAD przy staged | Rewizja, z której następuje przywrócenie. |
| `staged` | `bool` | `false` | Przywraca indeks. |
| `worktree` | `bool` | true albo false przy staged | Przywraca drzewo robocze. |
| `hunks` | `list<text>` | brak | Tylko te fragmenty zmian, według identyfikatora jak w git.stage. |
| `checkpoint` | `bool` | `false` | Tworzy punkt kontrolny zmian przed ich odrzuceniem. |
| `expect_state` | `text` | brak | Token stanu z wcześniejszego odczytu; nic się nie zmienia, gdy stan repozytorium jest inny. |

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

Odrzuć lokalne zmiany jednego pliku.

```nurt example
git.restore "/home/user/repo" paths: ["/home/user/repo/src/main.rs"]
```

```text expected
effect git.restore written: /home/user/repo/src/main.rs
```

Usuń z indeksu każdą zmianę pod katalogiem, zachowując drzewo robocze.

```nurt example
git.restore "/home/user/repo" paths: ["/home/user/repo/docs"] staged: true worktree: false
```

Przywróć pliki katalogu ze starszej rewizji do indeksu i drzewa roboczego.

```nurt example
git.restore "/home/user/repo" paths: ["/home/user/repo/config"] source: "HEAD~2" staged: true worktree: true
```

Cofnij drugi fragment zmian pliku, zachowując jego pozostałe zmiany lokalne.

```nurt example
git.restore "/home/user/repo" paths: ["/home/user/repo/src/main.rs"] hunks: ["src/main.rs#2"]
```

Usuń z indeksu jeden fragment zmian i zachowaj punkt kontrolny stanu sprzed tej operacji.

```nurt example
git.restore "/home/user/repo" paths: ["/home/user/repo/src/lib.rs"] hunks: ["src/lib.rs#1"] staged: true worktree: false checkpoint: true
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NO_MATCH` | Pathspec nie pasuje do żadnego pliku źródła ani indeksu. | Sprawdź ścieżkę poleceniem `git.status "/home/user/repo"` |
| `E_GIT_REVISION` | Źródło nie istnieje albo nie jest drzewem. | Podaj istniejącą rewizję, znacznik lub drzewo |
| `E_GIT_DIRTY` | Wybrana ścieżka ma nierozwiązane konflikty (`unmerged`). | Przywróć z `staged: true source: "HEAD"` |
| `E_ARG` | Nie ustawiono ani `staged`, ani `worktree`, pathspec lub identyfikator fragmentu jest nieprawidłowy albo fragment leży poza `paths` (`hunk_outside_paths`). | Ustaw `worktree: true` albo dodaj plik fragmentu do `paths` |
| `E_INVALID_VALUE` | Ścieżka bezwzględna leży poza repozytorium (`outside_repository`). | Podaj ścieżkę wewnątrz drzewa roboczego |
| `E_GIT_UNSUPPORTED` | `hunks` razem z `source` (`hunks_source`), z obiema stronami (`hunks_both`) albo dla podmodułu (`hunks_submodule`). | Usuń `source` albo ustaw `worktree: false` dla fragmentów w indeksie |
| `E_NOT_FOUND` | Numer fragmentu lub zmiana fragmentu nie istnieje (`not_found`). | Wyświetl fragmenty ponownie poleceniem `git.stage "/home/user/repo" action: list` |
| `E_GIT_LOCKED` | Istnieje `index.lock`. | Zaczekaj na inny proces albo usuń nieaktualną blokadę |
| `E_STALE` | Plik zmienił się między planowaniem a zapisem, odcisk fragmentu już nie pasuje (`changed`) albo stan różni się od `expect_state` (`state_changed`). | Odczytaj stan lub fragmenty ponownie i powtórz polecenie |

## Powiązane {#related}

`git.switch`, `git.stage`, `git.reset`, `git.checkpoint`, `undo`.
