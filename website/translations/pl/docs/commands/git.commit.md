---
command: git.commit
group: git
summary: Zapisuje rewizję i zgłasza jej gałąź oraz pliki.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.commit

## Składnia {#synopsis}

```text
git.commit <repo> message: <text> [name: value]...
```

## Opis {#description}

Zapisuje rewizję na gałęzi, na którą wskazuje `HEAD`, albo na samym `HEAD`, gdy
jest odłączony, i zgłasza nową rewizję wraz z jej plikami.

Dokładnie jeden z parametrów `staged`, `all` i `paths` wybiera zawartość:

- `staged: true` zatwierdza indeks w takiej postaci, w jakiej jest. Gdy trwa
  scalanie (istnieje `MERGE_HEAD`), rewizja kończy scalanie: jej rodzicami są
  `HEAD` i rewizje z `MERGE_HEAD`, pliki `MERGE_HEAD`, `MERGE_MSG`,
  `MERGE_MODE` i `AUTO_MERGE` są usuwane, a wpis reflogu ma postać
  `commit (merge): <subject>`.
- `all: true` najpierw dodaje do poczekalni każdą zmianę drzewa roboczego:
  zmodyfikowane i usunięte pliki śledzone oraz pliki nieśledzone, które nie są
  ignorowane. Indeks jest zapisywany z zawartością poczekalni.
- `paths` zatwierdza `HEAD` z zawartością podanych plików i katalogów z drzewa
  roboczego, tak jak robi to `git commit --only`. Wpisy indeksu dla tych ścieżek
  są aktualizowane; pozostałe zmiany w poczekalni zostają w indeksie i poza
  rewizją. Ścieżka, która nie pasuje do żadnego pliku drzewa roboczego ani
  indeksu, jest błędem.

`amend: true` bez wyboru zawartości zastępuje tylko komunikat ostatniej
rewizji: drzewo i rodzice pozostają, a autor i czas autora są zachowane. Z
wyborem zawartości `amend` zastępuje ostatnią rewizję rewizją o wybranej
zawartości z tymi samymi rodzicami. Poprawianie jest odrzucane, gdy trwa
scalanie.

Komunikat jest uzupełniany o znak nowego wiersza, a następnie o
`Signed-off-by: <committer>` z `sign_off` i o `trailers`, które dołączają do
istniejącego bloku stopek albo zaczynają nowy po pustym wierszu; stopka równa
ostatniej stopce bloku nie jest powtarzana. Haki `prepare-commit-msg` i
`commit-msg` mogą zmienić komunikat; wynik jest oczyszczany zgodnie z
`commit.cleanup` (`strip` usuwa także wiersze zaczynające się od
`core.commentChar`). Autor pochodzi z `author`, `GIT_AUTHOR_NAME`,
`GIT_AUTHOR_EMAIL`, `author.*`, `user.*` i `EMAIL`; wprowadzający rewizję
(committer) z `GIT_COMMITTER_*`, `committer.*`, `user.*` i `EMAIL`, a w ostateczności z
części `author`. Daty pochodzą z `GIT_AUTHOR_DATE` i `GIT_COMMITTER_DATE`, a w
przeciwnym razie z zegara z przesunięciem czasu hosta. `i18n.commitEncoding`
inne niż UTF-8 jest zapisywane w nagłówku `encoding`. Z `sign: true` albo bez
`sign`, gdy ustawiono `commit.gpgSign`, rewizja jest podpisywana kluczem
`user.signingKey` w formacie `gpg.format` (`openpgp` lub `ssh`). Podpis OpenPGP
tworzy GnuPG: program wskazany przez `gpg.openpgp.program` lub `gpg.program`
(domyślnie `gpg`) jest uruchamiany przez usługę programów, tak jak `run`
uruchamia programy, z argumentami `git commit -S` i rewizją na standardowym
wejściu. Klucz z `git.signing.key_file` jest importowany do tymczasowego
katalogu domowego GnuPG, który po podpisaniu jest usuwany wraz ze swoim agentem;
hasło pochodzi z pliku wskazanego przez `NURT_GPG_PASSPHRASE_FILE`. Brakujący
lub zawodzący program powoduje odrzucenie rewizji z błędem `E_GIT_SIGN` i
problemem `program_unavailable` lub `program_failed`, a nic nie jest zapisywane.
Podpis SSH jest tworzony w procesie, bez uruchamiania `ssh-keygen`;
`gpg.ssh.program` nie jest uruchamiany. `sign: false` pozostawia rewizję
niepodpisaną, niezależnie od `commit.gpgSign`.

Rewizja, której drzewo jest równe drzewu jej pierwszego rodzica (puste drzewo
dla pierwszej rewizji gałęzi), jest odrzucana, chyba że `allow_empty` ma
wartość true albo rewizja kończy scalanie. `exclude` pomija pasujące ścieżki:
ich zmiany nie są dodawane do poczekalni, a rewizja zapisuje ich wersję z
`HEAD`; raport je zlicza.

Haki `pre-commit`, `prepare-commit-msg`, `commit-msg` i `post-commit` z
`core.hooksPath` albo z `hooks/` w katalogu repozytorium są uruchamiane jako
programy użytkownika w katalogu głównym drzewa roboczego, z limitem
`git.hooks.timeout`. Otrzymują kopię indeksu (`GIT_INDEX_FILE`) i komunikatu w
katalogu roboczym poza repozytorium; zawartość, którą hak `pre-commit` doda do
poczekalni w kopii, staje się częścią rewizji. Zawodzący hak `post-commit` nie
zmienia wyniku.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `message` | `text` | wymagany | Komunikat rewizji. |
| `paths` | `list<path>` | brak | Pliki do dodania do poczekalni przed zapisaniem rewizji. |
| `all` | `bool` | `false` | Dodaje do poczekalni wszystkie zmiany plików śledzonych przed zapisaniem rewizji. |
| `staged` | `bool` | `false` | Zatwierdza dokładnie zawartość indeksu. |
| `amend` | `bool` | `false` | Zastępuje ostatnią rewizję. |
| `trailers` | `list<text>` | `[]` | Stopki dołączane do komunikatu, takie jak wiersze Co-Authored-By. |
| `sign_off` | `bool` | `false` | Dołącza stopkę Signed-off-by. |
| `author` | `text` | konfiguracja repozytorium | Autor w postaci Name <email>. |
| `allow_empty` | `bool` | `false` | Zezwala na rewizję bez zmian. |
| `hooks` | `bool` | `true` | Uruchamia haki pre-commit, prepare-commit-msg, commit-msg i post-commit repozytorium jako programy użytkownika. |
| `sign` | `bool` | ustawienie commit.gpgSign konfiguracji repozytorium | Podpisuje rewizję kluczem podpisującym z konfiguracji repozytorium. |
| `exclude` | `list<glob>` | `[]` | Ścieżki pomijane w rewizji; raport zlicza wykluczone pliki. |
| `expect_state` | `text` | brak | Token stanu z wcześniejszego odczytu; nic się nie zmienia, gdy stan repozytorium jest inny. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect`, `commit` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |
| Warunki odwracalności | Cofnięcie przesuwa odwołanie gałęzi wstecz; drzewo robocze nie jest zmieniane, a zapisane obiekty pozostają w repozytorium. |
| External use | `user_program`: `gpg`; Uruchamia gpg (gpg.openpgp.program lub gpg.program), gdy rewizja jest podpisywana kluczem OpenPGP, niezależnie od tego, czy podano sign, czy ustawiono commit.gpgSign; uruchamia haki pre-commit, prepare-commit-msg, commit-msg i post-commit repozytorium jako programy użytkownika, gdy podano hooks. |

## Przykłady {#examples}

Zapisanie rewizji ze wszystkimi zmianami drzewa roboczego i z podpisem Signed-off-by.

```nurt example
git.commit "/home/user/repo" message: "feat: add the parser" all: true sign_off: true
```

```text expected
effect kind: git.commit status: ok
commit short_id: 3f2a9c1 subject: "feat: add the parser" refs: [main]
```

Zapisanie rewizji tylko z dwoma plikami, z pozostawieniem innych zmian z poczekalni w indeksie.

```nurt example
git.commit "/home/user/repo" message: "fix: handle empty input" paths: ["/home/user/repo/src/parse.rs", "/home/user/repo/tests/parse.rs"]
```

Dodanie stopki współautora do zmian w poczekalni z pominięciem haków.

```nurt example
git.commit "/home/user/repo" message: "docs: describe options" staged: true trailers: ["Co-Authored-By: Ada Lovelace <ada@example.com>"] hooks: false
```

Zmiana sformułowania komunikatu ostatniej rewizji.

```nurt example
git.commit "/home/user/repo" message: "fix: handle empty input files" amend: true
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_PARAM_CONFLICT` | Nie podano żadnego albo podano więcej niż jeden z `staged`, `all`, `paths` (problemy `missing_selection`, `selection_conflict`), albo użyto `amend` lub `paths` podczas scalania. | Podaj jeden wybór; zakończ scalanie za pomocą `staged: true` |
| `E_GIT_NOTHING_TO_COMMIT` | Rewizja nie zapisałaby żadnej zmiany (problem `nothing_to_commit`, `amend_empty`). | Dodaj zmiany do poczekalni, użyj `all: true` lub `allow_empty: true` |
| `E_GIT_IN_PROGRESS` | Indeks ma nierozwiązane konflikty (problem `unmerged_paths`). | Rozwiąż konflikty i dodaj pliki do poczekalni poleceniem `git.stage` |
| `E_CONFIG` | Brak tożsamości autora lub wprowadzającego rewizję (problem `missing_identity`) albo `commit.cleanup` ma nieprawidłową wartość. | Ustaw `user.name` i `user.email` albo podaj `author` |
| `E_INVALID_VALUE` | `author` nie ma postaci `Name <email>`, stopka jest niepoprawna, zmienna daty jest nieprawidłowa albo komunikat jest pusty po oczyszczeniu. | Popraw wartość |
| `E_NOT_FOUND` | Ścieżka z `paths` nie pasuje do żadnego pliku (problem `path_not_found`). | Sprawdź pisownię poleceniem `git.status` |
| `E_GIT_HOOK` | `pre-commit`, `prepare-commit-msg` lub `commit-msg` zakończył się niepowodzeniem; błąd zawiera ostatnie wiersze jego wyjścia. | Popraw zgłoszony problem albo użyj `hooks: false` |
| `E_TIMEOUT` | Hak przekroczył `git.hooks.timeout`. | Podnieś limit albo pomiń haki |
| `E_GIT_SIGN` | Nie można podpisać rewizji (`no_key`, `agent_unavailable`, `key_locked`, `unsupported_format`). | Skonfiguruj `user.signingKey` albo uruchom agenta kluczy |
| `E_STALE` | `expect_state` różni się od stanu repozytorium (problem `state_changed`) albo plik zmienił się podczas zapisywania rewizji. | Odczytaj stan ponownie i powtórz wywołanie |
| `E_GIT_REF` | Gałąź przesunęła się po zaplanowaniu. | Powtórz wywołanie |
| `E_GIT_LOCKED` | Istnieje plik blokady `git`. | Poczekaj na inny proces `git` albo usuń nieaktualną blokadę |
| `E_GIT_UNSUPPORTED` | `all` lub `paths` w repozytorium typu bare (problem `bare_repository`). | Użyj `staged: true` |

## Powiązane {#related}

`git.status`, `git.stage`, `git.review`, `git.log`, `undo`.
