---
command: git.clone
group: git
summary: Klonuje repozytorium przez HTTPS lub SSH do nowego katalogu.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.clone

## Składnia {#synopsis}

```text
git.clone <url> <to> [name: value]...
```

## Opis {#description}

Klonuje repozytorium do katalogu, który nie istnieje, tak jak robi to
`git clone`.

`url` to adres osiągany przez smart HTTP (`http://`, `https://`), SSH
(`ssh://`, `user@host:path`), adres `file://` albo bezwzględna ścieżka lokalna.
Względna ścieżka lokalna jest odrzucana. `to` nie może istnieć, a jego katalog
nadrzędny musi istnieć.

Wynik ma układ, który tworzy `git clone`:

- repozytorium zdalne `origin` z adresem oraz, w repozytorium z drzewem
  roboczym, specyfikacją odwołań `+refs/heads/*:refs/remotes/origin/*`;
- gałęzie repozytorium zdalnego jako `refs/remotes/origin/*` oraz tagi, w
  `packed-refs`;
- `refs/remotes/origin/HEAD` wskazujące domyślną gałąź repozytorium zdalnego;
- gałąź lokalna `branch` albo domyślna gałąź repozytorium zdalnego, z
  `branch.<name>.remote` i `branch.<name>.merge`, wyewidencjonowana do drzewa
  roboczego i indeksu;
- komunikat reflogu `clone: from <address>` dla `HEAD`, gałęzi i
  `refs/remotes/origin/HEAD`.

Repozytorium zdalne z odłączonym `HEAD` daje odłączony `HEAD`. Repozytorium
zdalne bez rewizji daje nienarodzoną gałąź nazwaną jak jego gałąź domyślna.

Z `depth` pobierana jest tylko wybrana gałąź, jej historia jest ograniczona do
tylu rewizji, `shallow` zapisuje granicę, specyfikacja odwołań wskazuje jedną
gałąź, a zapisywane są tylko tagi wskazujące na pobraną historię. Z
`bare: true` repozytorium nie ma drzewa roboczego, gałęzie repozytorium
zdalnego stają się gałęziami lokalnymi, a specyfikacja odwołań i reflog nie są
zapisywane.

Adres zapisany w `remote.origin.url` nie zawiera hasła. Poświadczenia dla HTTP
są pobierane z adresu, ze zmiennych `NURT_GIT_TOKEN_<HOST>` i `NURT_GIT_TOKEN`,
ze zmiennych dostawców hostingu albo z magazynów poświadczeń; pomocniki
poświadczeń będące programami nie są uruchamiane.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `url` | `text` | wymagany | Adres repozytorium. |
| `to` | `path` | wymagany | Katalog do utworzenia. |
| `depth` | `int` | brak | Ogranicza sklonowaną historię do tylu rewizji. |
| `branch` | `text` | brak | Gałąź do wyewidencjonowania zamiast gałęzi domyślnej. |
| `bare` | `bool` | `false` | Tworzy repozytorium bez drzewa roboczego. |
| `accept_host_key` | `text` | brak | Odcisk SHA-256 (SHA256:<base64>) klucza hosta SSH do zaakceptowania, gdy host nie jest znany. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect`, `git_ref_update` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `external` |
| Odwracalność | `irreversible` |
| Implementacja | `builtin` |
| Warunki odwracalności | Nowy katalog jest usuwany przez undo; transferu sieciowego nie można odwrócić. |

## Przykłady {#examples}

Klonowanie domyślnej gałęzi repozytorium.

```nurt example
git.clone "https://example.com/org/app.git" "/home/user/app"
```

```text expected
effect  git.clone  ok  /home/user/app  state: completed
```

Klonowanie tylko ostatniej rewizji gałęzi `release`.

```nurt example
git.clone "https://example.com/org/app.git" "/home/user/app" branch: "release" depth: 1
```

Utworzenie repozytorium bare ze ścieżki lokalnej.

```nurt example
git.clone "/srv/git/app.git" "/home/user/app.git" bare: true
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_EXISTS` | `to` istnieje (`destination_exists`). | Przekaż katalog, który nie istnieje. |
| `E_NOT_FOUND` | Katalog nadrzędny `to` nie istnieje (`parent_missing`). | Najpierw utwórz katalog nadrzędny. |
| `E_INVALID_VALUE` | `depth` wynosi 0 albo `url` jest pusty lub jest względną ścieżką lokalną. | Przekaż `depth` równe 1 lub więcej oraz ścieżkę bezwzględną albo adres `file://`. |
| `E_GIT_REF` | `branch` nie jest gałęzią repozytorium zdalnego (`remote_branch_not_found`). | Przekaż jedną z gałęzi wymienionych w błędzie albo pomiń `branch`. |
| `E_GIT_AUTH` | Repozytorium zdalne wymaga poświadczeń, których brakuje lub które zostały odrzucone. | Ustaw `NURT_GIT_TOKEN_<HOST>` i użyj adresu `https://`. |
| `E_GIT_HOST_KEY` | Klucz hosta SSH jest nieznany lub się zmienił. | Przekaż `accept_host_key` z odciskiem hosta. |
| `E_GIT_PROTOCOL` | Repozytorium zdalne odpowiedziało niezgodnie z protokołem. | Sprawdź, czy adres wskazuje repozytorium Git. |
| `E_NETWORK` | Połączenie nie powiodło się; problem to `dns`, `connect`, `tls`, `timeout`, `proxy`, `http_status` lub `insecure_credentials` (nagłówek `Authorization` albo token zostałby wysłany przez `http://` do hosta innego niż ta maszyna). | Sprawdź adres, ustawienia proxy i sieć; użyj adresu `https://`. |
| `E_CANCELLED` | Transfer został przerwany; nowy katalog został usunięty. | Powtórz wywołanie. |
| `E_TIMEOUT` | Transfer został przerwany; nowy katalog został usunięty. | Powtórz wywołanie. |

## Powiązane {#related}

`git.fetch`, `git.switch`, `git.worktree`, `git.status`, `undo`.
