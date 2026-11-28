---
command: git.pull
group: git
summary: Pobiera zmiany z repozytorium zdalnego i włącza je do bieżącej gałęzi.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.pull

## Składnia {#synopsis}

```text
git.pull <repo> [name: value]...
```

## Opis {#description}

Pobiera zmiany z repozytorium zdalnego i włącza pobraną gałąź do bieżącej
gałęzi, tak jak robi to `git pull`.

Repozytorium zdalnym jest `remote`, a w jego braku `branch.<name>.remote`
bieżącej gałęzi, a w jego braku `origin`. Gałęzią do włączenia jest `branch`, a
w jego braku gałąź upstream bieżącej gałęzi (`branch.<name>.merge`). Bez
`branch` repozytorium zdalne musi być repozytorium zdalnym upstream bieżącej
gałęzi.

Włączanie zależy od `mode`. Bez `mode` decyduje konfiguracja repozytorium:
`branch.<name>.rebase`, a następnie `pull.rebase` wybierają rebase (`true`,
`merges`, `interactive`) albo scalanie (`false`); `pull.ff` wybiera politykę
fast-forward (`true`, `false`, `only`). Kroki są następujące:

- Gdy bieżąca gałąź jest zawarta w pobranej gałęzi, gałąź jest przesuwana w
  trybie fast-forward, także gdy skonfigurowano rebase.
- Gdy pobrana gałąź jest zawarta w bieżącej gałęzi, nic się nie zmienia, a stan
  to `up_to_date`.
- Gdy historie się rozeszły, `mode: merge` tworzy rewizję scalającą z
  komunikatem `Merge branch '<branch>' of <url>`, a po nim ` into <current>`,
  chyba że bieżąca gałąź pasuje do `merge.suppressDest` (domyślnie `main` i
  `master`); `mode: rebase` odtwarza lokalne rewizje na pobranej gałęzi,
  zaczynając po punkcie rozwidlenia zapisanym w reflogu gałęzi śledzącej
  repozytorium zdalne; `mode: fast_forward_only` i `pull.ff=only` odrzucają
  operację.
- Gdy ani `mode`, ani konfiguracja nie dokonują wyboru, rozeszłe historie są
  odrzucane z błędem `E_GIT_NON_FAST_FORWARD` (problem `diverged`), jak w
  Git 2.53, a błąd proponuje `mode: merge` i `mode: rebase`.
- Na gałęzi bez rewizji pobrana rewizja jest wyewidencjonowywana, a gałąź jest
  tworzona z komunikatem reflogu `initial pull`.

Sprawdzenia, które nie wymagają repozytorium zdalnego, są wykonywane przed
transferem i niczego nie zmieniają: pliki niescalone, trwające scalanie lub
rebase oraz, dla rebase bez `rebase.autoStash`, zmiany spoza poczekalni lub w
poczekalni. Zmiany lokalne i pliki nieśledzone stojące na drodze fast-forward
lub scalania są wykrywane po transferze; wówczas, jak w Git, pobrane odwołania i
`FETCH_HEAD` są aktualizowane, a drzewo robocze i bieżąca gałąź pozostają bez
zmian.

Scalanie z konfliktami zatrzymuje się tak jak `git.merge`: drzewo robocze
zawiera znaczniki konfliktów, indeks zawiera etapy konfliktów, a `MERGE_HEAD`,
`MERGE_MSG`, `MERGE_MODE`, `AUTO_MERGE` i `ORIG_HEAD` są zapisywane. Znaczniki
konfliktów wskazują scalaną rewizję przez jej pełny identyfikator, tak jak Git
robi to przy pull.

Programy wskazane przez konfigurację (pomocniki poświadczeń, `core.sshCommand`,
filtry zawartości) nie są uruchamiane, chyba że pozwala na to odpowiednie
ustawienie konsoli; pominięte programy są zgłaszane jako wpisy uwag.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `remote` | `text` | `"origin"` | Repozytorium zdalne, z którego wykonujemy pull. |
| `branch` | `text` | upstream bieżącej gałęzi | Gałąź repozytorium zdalnego do włączenia. |
| `mode` | `enum(merge\|rebase\|fast_forward_only)` | ustawienie konfiguracji repozytorium | Sposób włączania zmian. |
| `accept_host_key` | `text` | brak | Odcisk SHA-256 (SHA256:<base64>) klucza hosta SSH do zaakceptowania, gdy host nie jest znany. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect`, `git_ref_update`, `git_conflict` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `external` |
| Odwracalność | `irreversible` |
| Implementacja | `builtin` |
| Warunki odwracalności | Zmiany lokalne w drzewie roboczym są zachowane w dzienniku; transferu sieciowego nie można odwrócić. |

## Przykłady {#examples}

Pull gałęzi upstream bieżącej gałęzi ze skonfigurowanym trybem.

```nurt example
git.pull "/home/user/app"
```

```text expected
effect  git.pull  ok  remote:origin  state: fast_forward
```

Pull gałęzi `release` repozytorium zdalnego `upstream` z utworzeniem rewizji
scalającej, gdy historie się rozeszły.

```nurt example
git.pull "/home/user/app" remote: "upstream" branch: "release" mode: merge
```

Odtworzenie lokalnych rewizji na gałęzi upstream.

```nurt example
git.pull "/home/user/app" mode: rebase
```

Aktualizacja bieżącej gałęzi tylko wtedy, gdy scalanie nie jest potrzebne, wraz
z zaakceptowaniem klucza nowego hosta SSH.

```nurt example
git.pull "/home/user/app" mode: fast_forward_only accept_host_key: "SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `repo` nie znajduje się w repozytorium. | Podaj ścieżkę repozytorium. |
| `E_GIT_IN_PROGRESS` | Pliki niescalone albo trwające scalanie lub rebase. | `git.merge "<repo>" action: abort`, `git.rebase "<repo>" action: continue` |
| `E_GIT_DIRTY` | Niezapisane zmiany blokują rebase (przed transferem) albo zmiany lokalne zostałyby nadpisane (po transferze, `effect_applied`). | `git.stash "<repo>" action: push` |
| `E_GIT_NON_FAST_FORWARD` | Historie rozeszły się bez wybranego trybu (`diverged`) albo dozwolony jest tylko fast-forward (`not_fast_forward`). | `git.pull "<repo>" mode: merge` lub `mode: rebase` |
| `E_GIT_REF` | Brak gałęzi upstream (`no_upstream`), odłączony `HEAD`, repozytorium zdalne inne niż upstream bez `branch` albo gałąź nie istnieje w repozytorium zdalnym. | `git.pull "<repo>" remote: "origin" branch: "<name>"` |
| `E_GIT_CONFLICT` | Scalanie zatrzymało się z konfliktami; `effect_applied` ma wartość true. | Rozwiąż konflikty, następnie `git.merge "<repo>" action: continue` |
| `E_GIT_UNSUPPORTED` | Nieprawidłowa wartość `pull.rebase` lub `pull.ff`, repozytorium typu bare, kilka gałęzi do scalenia naraz albo rebase, którego instalacja nie udostępnia. | Popraw konfigurację albo podaj `mode`. |
| `E_GIT_LOCKED` | Istnieje plik blokady indeksu lub odwołania. | Usuń nieaktualny plik `.lock` po sprawdzeniu, że nie działa żaden proces Git. |
| `E_NETWORK` | Transfer nie powiódł się; nic lokalnie się nie zmieniło. | Zobacz `git.fetch`. |
| `E_GIT_AUTH` | Transfer nie powiódł się; nic lokalnie się nie zmieniło. | Zobacz `git.fetch`. |
| `E_GIT_HOST_KEY` | Transfer nie powiódł się; nic lokalnie się nie zmieniło. | Zobacz `git.fetch`. |
| `E_GIT_PROTOCOL` | Transfer nie powiódł się; nic lokalnie się nie zmieniło. | Zobacz `git.fetch`. |

Błędy po transferze zawierają `effect_applied: true` i wymieniają
zaktualizowane odwołania śledzące repozytorium zdalne w `details.ref_updates`.

## Powiązane {#related}

`git.fetch`, `git.merge`, `git.rebase`, `git.push`, `git.status`.
