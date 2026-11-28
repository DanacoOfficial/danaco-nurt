---
command: git.tag
group: git
summary: Wyświetla znaczniki albo tworzy lub usuwa znacznik.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.tag

## Składnia {#synopsis}

```text
git.tag <repo> [name: value]...
```

## Opis {#description}

Bez `name` i `delete` polecenie wyświetla znaczniki w kolejności
`git tag --list`. Znacznik opisany raportuje swojego autora znacznika, czas,
komunikat i obiekt, na który wskazuje; znacznik lekki raportuje tylko swój cel.

`name` tworzy `refs/tags/<name>` na `revision`. Bez `message` znacznik jest
lekki. Przy `message` najpierw zapisywany jest obiekt znacznika opisanego;
komunikat jest czyszczony tak, jak czyści go `git tag -m` (usunięte końcowe białe
znaki, zwinięte ciągi pustych wierszy, usunięte początkowe i końcowe puste
wiersze, jeden końcowy znak nowego wiersza), a obiekt jest identyczny bajt w
bajt z obiektem, który `git tag -a -m` zapisuje dla tego samego autora znacznika
i czasu. Autorem znacznika jest tożsamość osoby zatwierdzającej repozytorium:
`GIT_COMMITTER_NAME`, `GIT_COMMITTER_EMAIL` i `GIT_COMMITTER_DATE`, następnie
`user.name` i `user.email`, a w końcu zegar wykonania.

Przy `sign: true` znacznik opisany jest podpisywany kluczem wskazanym przez
`user.signingKey` w formacie `gpg.format` (`openpgp` lub `ssh`); bez `sign`
decyduje ustawienie `tag.gpgSign`, a `sign: false` tworzy znacznik niepodpisany,
gdy `tag.gpgSign` ma wartość `true`. Podpis OpenPGP jest tworzony przez program
wskazany przez `gpg.openpgp.program` lub `gpg.program` (domyślnie `gpg`),
uruchamiany przez usługę programów tak jak w `git.commit`; podpis SSH jest
tworzony w procesie, a `gpg.ssh.program` i podobne klucze nie są uruchamiane.
Znacznik podpisany wymaga komunikatu; znacznik bez komunikatu pozostaje lekki
i niepodpisany.

`delete` usuwa odwołanie znacznika, z jego pliku luźnego i z `packed-refs`;
obiekt znacznika pozostaje w bazie obiektów.

Dziennik odwołań znacznika jest zapisywany tylko wtedy, gdy `core.logAllRefUpdates`
ma wartość `always`, z komunikatem `tag: tagging <id> (<subject>, <date>)`
z `git tag`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `name` | `text` | brak | Nazwa znacznika do utworzenia. |
| `revision` | `text` | `"HEAD"` | Rewizja, na którą wskazuje nowy znacznik. |
| `message` | `text` | brak | Komunikat znacznika opisanego. |
| `sign` | `bool` | ustawienie tag.gpgSign z konfiguracji repozytorium | Tworzy podpisany znacznik opisany kluczem podpisującym z konfiguracji repozytorium. |
| `delete` | `text` | brak | Znacznik do usunięcia. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `git_tag`, `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |
| Warunki odwracalności | Zmiany znaczników są odwracalne z dziennika, który przechowuje poprzednią wartość każdego odwołania. |
| External use | `user_program`: `gpg`; Uruchamia gpg (gpg.openpgp.program lub gpg.program), gdy znacznik opisany jest podpisywany kluczem OpenPGP, niezależnie od tego, czy podano sign, czy ustawiono tag.gpgSign. |

| Akcja | Skutek | Odwracalność | Uwaga |
|---|---|---|---|
| `name` | `local` | `reversible` | Obowiązuje, gdy podano name; tworzy znacznik. |
| `delete` | `local` | `reversible` | Obowiązuje, gdy podano delete. |

## Przykłady {#examples}

Wyświetl znaczniki.

```nurt example
git.tag "/home/user/repo"
```

```text expected
git_tag name: v1.2.0 kind: annotated target: 4f2a9c1
```

Utwórz znacznik lekki na bieżącej rewizji.

```nurt example
git.tag "/home/user/repo" name: "build-417"
```

Utwórz znacznik opisany na innej rewizji.

```nurt example
git.tag "/home/user/repo" name: "v1.3.0" revision: "release/1.3" message: "Release 1.3.0"
```

Utwórz podpisany znacznik opisany.

```nurt example
git.tag "/home/user/repo" name: "v1.3.1" message: "Release 1.3.1" sign: true
```

Usuń znacznik.

```nurt example
git.tag "/home/user/repo" delete: "build-417"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_GIT_REF` | Nazwa jest nieprawidłowa (`invalid_name`), znacznik istnieje (`exists`) albo nie istnieje (`missing`). | `git.tag "/home/user/repo" delete: "v1.3.0"` |
| `E_GIT_REVISION` | Nie można rozwiązać `revision`. | `git.log "/home/user/repo"` |
| `E_GIT_SIGN` | Znacznik jest podpisywany, a klucz jest niedostępny (`signer_unavailable`) albo podpisywanie się nie powiodło. | Skonfiguruj `user.signingKey` albo przekaż `sign: false` |
| `E_MISSING_PARAM` | `sign: true` bez `message` (`missing_message`). | Dodaj `message` |
| `E_GIT_LOCKED` | Istnieje blokada odwołania. | Zaczekaj na inny proces albo usuń nieaktualną blokadę |
| `E_STALE` | Odwołanie zmieniło się między planowaniem a zapisem. | Uruchom polecenie ponownie |
| `E_ARG` | Podano jednocześnie `name` i `delete` (`param_conflict`). | Przekaż jeden z nich |

## Powiązane {#related}

`git.branch`, `git.log`, `git.show`, `undo`.
