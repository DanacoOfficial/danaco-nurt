---
command: git.checkpoint
group: git
summary: Tworzy, wyświetla, porównuje, przywraca lub usuwa punkty kontrolne drzewa roboczego i indeksu przechowywane poza gałęziami.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.checkpoint

## Składnia {#synopsis}

```text
git.checkpoint <repo> action: <enum(create|list|compare|restore|delete)> [name: value]...
```

## Opis {#description}

Punkt kontrolny to rewizja, która zapisuje drzewo robocze tak, jak umieściłoby
je w poczekalni `git add -A` w tymczasowym indeksie: pliki śledzone wraz z ich
zawartością w drzewie roboczym oraz pliki nieśledzone, które nie są
ignorowane. Pliki ignorowane nie są zapisywane. Z `index: true` druga rewizja
zapisuje indeks i staje się ostatnim rodzicem rewizji punktu kontrolnego, jak w
wpisie stash. Pierwszym rodzicem jest `HEAD`; repozytorium bez rewizji daje
punkt kontrolny bez rodziców.

Punkty kontrolne są przechowywane jako `refs/nurt/checkpoints/cp-<N>`,
numerowane od 1 w rosnącej kolejności. Utworzenie punktu kontrolnego nie
zmienia `HEAD`, indeksu, gałęzi ani drzewa roboczego. Komunikat rewizji ma
postać

```text
nurt checkpoint cp-3: before refactoring

branch: main
head: 5b6f0e3c0a2d7c1b0d1e9f6a4c3b2a1908f7e6d5
index: none
```

Punkty kontrolne w `refs/danaco/checkpoints/` są wyświetlane, porównywane i
przywracane, ale nie są usuwane; nowe numery kontynuują numerację po
najwyższym numerze z obu przedrostków.

`list` zwraca punkty kontrolne od najnowszego. `compare` oblicza różnicę między
punktem kontrolnym a stroną `against`: innym punktem kontrolnym (`cp-<N>`),
dowolną wersją albo drzewem roboczym, gdy `against` jest pominięte. Fragmenty
różnic (hunk) używają algorytmu Myersa z heurystyką wcięć `git diff`. `restore`
zapisuje pliki punktu kontrolnego, które różnią się od drzewa roboczego,
usuwa pliki śledzone i nieśledzone, które nie są ignorowane i nie ma ich w
punkcie kontrolnym, oraz usuwa katalogi pozostawione puste przez te usunięcia.
`HEAD` i gałęzie nie zmieniają się. Z `index: true` wpisy indeksu w obrębie
`paths` są zastępowane zapisanym indeksem. `delete` usuwa wskazane punkty
kontrolne albo wszystkie poza `keep` najnowszymi.

Sterowniki filtrów zawartości skonfigurowane przez repozytorium nie są
uruchamiane; konwersja zakończeń wierszy, `ident` i `working-tree-encoding` są
stosowane.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `action` | `enum(create\|list\|compare\|restore\|delete)` | wymagany | Operacja na punktach kontrolnych. |
| `label` | `text` | brak | Etykieta nowego punktu kontrolnego. |
| `checkpoint_id` | `text \| list<text>` | brak | Punkt kontrolny do porównania lub przywrócenia albo punkty kontrolne do usunięcia. |
| `against` | `text` | drzewo robocze | Z compare: wersja lub punkt kontrolny, z którym porównujemy. |
| `stat` | `bool` | `false` | Z compare: tylko zmienione pliki z liczbą dodanych i usuniętych wierszy. |
| `context` | `int` | `3` | Z compare: wiersze kontekstu wokół każdej zmiany. |
| `function_context` | `bool` | `false` | Z compare: rozszerza każdą zmianę na całą otaczającą funkcję. |
| `symbol` | `text` | brak | Z compare: tylko zmiany w obrębie tego elementu kodu. |
| `word_diff` | `bool` | `false` | Z compare: pokazuje zmiany słowo po słowie. |
| `ignore_whitespace` | `bool` | `false` | Z compare: ignoruje zmiany w białych znakach. |
| `ignore_moved` | `bool` | `false` | Z compare: ukrywa bloki wierszy, które zostały tylko przeniesione. |
| `paths` | `list<path>` | brak | Tylko te pliki lub katalogi. |
| `index` | `bool` | `false` | Z restore: przywraca także indeks. |
| `keep` | `int` | brak | Z delete: zachowuje tylko tyle najnowszych punktów kontrolnych. |
| `save_current` | `bool` | `true` | Tworzy punkt kontrolny bieżących zmian przed restore. |
| `exclude` | `list<glob>` | `[]` | Ścieżki pomijane przy create; raport zlicza wykluczone pliki. |
| `expect_state` | `text` | brak | Token stanu z wcześniejszego odczytu; nic się nie zmienia, gdy stan repozytorium jest inny. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `git_checkpoint`, `hunk`, `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |
| Warunki odwracalności | Restore zachowuje dokładną poprzednią zawartość każdego pliku, który zmienia lub usuwa. |

| Działanie | Skutek | Odwracalność | Uwaga |
|---|---|---|---|
| `list` | `read` | `read` |  |
| `compare` | `read` | `read` |  |

## Przykłady {#examples}

Zapisanie drzewa roboczego i indeksu przed zmianą.

```nurt example
git.checkpoint "/home/user/repo" action: create label: "before refactoring" index: true
```

```text expected
git_checkpoint id: cp-3 ref: refs/nurt/checkpoints/cp-3 label: before refactoring index_included: true
```

Pokazanie, które pliki zmieniły się od punktu kontrolnego.

```nurt example
git.checkpoint "/home/user/repo" action: compare checkpoint_id: "cp-3" stat: true
```

```text expected
change path: src/main.rs action: modify added: 4 removed: 1
```

Przywrócenie jednego katalogu z punktu kontrolnego.

```nurt example
git.checkpoint "/home/user/repo" action: restore checkpoint_id: "cp-3" paths: ["/home/user/repo/src"]
```

Zachowanie dziesięciu najnowszych punktów kontrolnych i usunięcie pozostałych.

```nurt example
git.checkpoint "/home/user/repo" action: delete keep: 10
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Brakuje `action`, `checkpoint_id` albo, przy `delete`, obu: `checkpoint_id` i `keep`. | `git.checkpoint "/home/user/repo" action: delete keep: 10` |
| `E_ARG` | Podano jednocześnie `checkpoint_id` i `keep` (`param_conflict`), identyfikator ma niepoprawną postać (`invalid_id`) albo ścieżka wskazuje katalog `.git`. | Przekaż jeden wybór; użyj postaci `cp-1` |
| `E_ARITY` | Do `compare` lub `restore` przekazano więcej niż jeden punkt kontrolny. | Przekaż jeden identyfikator |
| `E_GIT_REF` | Punkt kontrolny nie istnieje (`not_found`) albo został zapisany przez inne narzędzie (`read_only`). | `git.checkpoint "/home/user/repo" action: list` |
| `E_GIT_CHECKPOINT` | `index: true` zażądano dla punktu kontrolnego bez zapisanego indeksu (`no_index`). | Przywróć bez `index` |
| `E_GIT_REVISION` | `against` nie wskazuje ani punktu kontrolnego, ani wersji. | Przekaż istniejącą wersję |
| `E_GIT_UNSUPPORTED` | Opcja porównania jest niedostępna (`diff_option`) albo repozytorium jest typu bare. | `git.diff` z odwołaniem do punktu kontrolnego |
| `E_GIT_NOT_REPOSITORY` | `repo` nie znajduje się w repozytorium albo ścieżka leży poza drzewem roboczym (`path_outside_repo`). | Przekaż ścieżkę wewnątrz drzewa roboczego |
| `E_GIT_LOCKED` | Odwołanie, `packed-refs` lub indeks jest zablokowany przez inny proces. | Poczekaj na inny proces albo usuń nieaktualny plik blokady |
| `E_STALE` | Plik, odwołanie lub indeks zmieniło się między planowaniem a zapisem. | Uruchom polecenie ponownie |

## Powiązane {#related}

`git.stash`, `git.diff`, `git.restore`, `undo`.
