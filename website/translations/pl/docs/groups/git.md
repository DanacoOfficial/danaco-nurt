---
group: git
summary: Odczytuje i zmienia repozytoria Git w ramach procesu, bez programu zewnętrznego.
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Grupa git

## Zakres {#scope}

Polecenia grupy `git` odczytują i zmieniają repozytoria Git w procesie
konsoli. Nie jest uruchamiany żaden program `git`, pomocnik poświadczeń,
klient SSH ani inny program wskazany przez konfigurację repozytorium. Haki
repozytorium i sterowniki filtrów to jedyne programy użytkownika, które te
polecenia uruchamiają, i działają one według reguł `run`.

Każda zmiana drzewa roboczego i katalogu repozytorium jest jedną operacją
dziennika i można ją cofnąć za pomocą `undo`. Obiekty dodane do magazynu
obiektów są zachowywane przez `undo`; `git gc` usuwa te, które stają się
nieosiągalne.

## Repozytorium {#repository}

Pierwszym parametrem każdego polecenia jest `repo: path`. Przyjmuje ono
korzeń drzewa roboczego, dowolny katalog pod nim, powiązane drzewo robocze,
katalog lub plik `.git` oraz repozytorium surowe (bare). Wyszukiwanie
repozytorium zatrzymuje się poniżej każdego katalogu wymienionego w
`GIT_CEILING_DIRECTORIES`.

Repozytorium należące do innego konta jest otwierane tylko wtedy, gdy
`safe.directory` je wymienia, tak jak w `git`; błąd podpowiada wpis do
dodania. Repozytorium, które używa `extensions.refStorage=reftable` lub
rozszerzenia, które nie jest zaimplementowane, kończy się błędem
`E_GIT_UNSUPPORTED`, zanim cokolwiek zostanie odczytane. `core.fsmonitor` nie
jest używane: pliki są sprawdzane bezpośrednio, a raport odnotowuje
`fsmonitor_ignored`.

## Ścieżki {#paths}

Ścieżki w parametrach są względne wobec korzenia drzewa roboczego, bez względu
na bieżący katalog. Ścieżka bezwzględna pod korzeniem jest akceptowana;
ścieżka poza nim kończy się błędem `E_INVALID_VALUE` (problem
`outside_repository`).

| Forma | Wybiera |
|---|---|
| `src` | plik `src` albo każdą ścieżkę pod katalogiem `src` |
| `src/*.rs` | ścieżki pasujące do wzorca glob; `*` pasuje także do `/` |
| `.` | każdą ścieżkę |

`exclude: list<glob>` usuwa ścieżki z wyboru ze składnią `.gitignore`: wzorzec
bez `/` pasuje do nazwy na dowolnej głębokości, wzorzec z `/` jest zakotwiczony
w korzeniu, `**` pasuje do dowolnej liczby katalogów, a wykluczony katalog
wyklucza wszystko pod nim.

## Konfiguracja {#configuration}

Konfiguracja jest odczytywana w kolejności `git`; późniejsza warstwa
nadpisuje wcześniejszą.

| Warstwa | Źródło |
|---|---|
| system | `GIT_CONFIG_SYSTEM` albo plik systemowy dystrybucji `git` (`/etc/gitconfig`; w systemie Windows `C:/Program Files/Git/etc/gitconfig`); brak, gdy `GIT_CONFIG_NOSYSTEM` ma wartość true |
| global | `GIT_CONFIG_GLOBAL` albo `$XDG_CONFIG_HOME/git/config`, a po nim `~/.gitconfig` |
| repozytorium | `config` katalogu repozytorium |
| drzewo robocze | `config.worktree`, gdy ustawiono `extensions.worktreeConfig` |
| środowisko | `GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_VALUE_<n>` |

`include` i `includeIf` są podążane. Zmienne środowiskowe są zmiennymi sesji
konsoli. Początkowe `~` w `core.excludesFile`, `core.attributesFile`,
`core.hooksPath` i `commit.template` oznacza katalog domowy sesji.

## Filtry zawartości {#content-filters}

Zawartość jest konwertowana między drzewem roboczym a magazynem obiektów tak,
jak konwertuje ją `git`: `core.autocrlf`, `core.eol` oraz atrybuty `text`,
`eol`, `crlf`, `binary`, `ident` i `working-tree-encoding`.

Sterownik wskazany atrybutem `filter` i skonfigurowany jako
`filter.<name>.clean`, `filter.<name>.smudge` lub `filter.<name>.process`
działa jako program użytkownika przez `sh -c`, z `%f` zastąpionym ujętą w
cudzysłów ścieżką. Gdy nie jest dostępna usługa programów albo sterownik,
który nie jest `filter.<name>.required`, zawodzi, zawartość przechodzi bez
filtrowania, a raport odnotowuje `filter_not_applied` z liczbą plików.
Zawodzący sterownik wymagany kończy polecenie błędem `E_EXTERNAL_FAILED`.
Plik wskaźnika Git LFS, który nie został skonwertowany, pozostaje plikiem
wskaźnika w drzewie roboczym.

## Rewizje {#revisions}

Wskazania rewizji (revisions) podlegają składni `git rev-parse`.

| Forma | Znaczenie |
|---|---|
| `main`, `v1.0`, `origin/main` | gałąź, znacznik lub gałąź śledząca zdalną |
| `3f2a9c1` | skrócony identyfikator obiektu |
| `HEAD~2`, `HEAD^2` | przodek, rodzic |
| `main@{1}`, `@{upstream}`, `@{push}` | wpis reflog, gałąź nadrzędna, cel wypychania |
| `:/fix parser` | najnowsza rewizja, której komunikat pasuje |
| `HEAD:src/main.rs` | obiekt pod ścieżką drzewa |
| `v1.0^{}`, `HEAD^{tree}` | obiekt po dereferencji |

Wskazanie rewizji, którego nie można rozstrzygnąć, kończy się błędem
`E_GIT_REVISION`. Jego problemem jest `not_found`, z odwołaniami o
najbliższej nazwie jako kandydatami, `ambiguous`, z pasującymi obiektami jako
kandydatami, `invalid_syntax` albo `not_a_commit` tam, gdzie wymagana jest
rewizja. Skrócone identyfikatory w wynikach mają długość `core.abbrev` i są
wydłużane, aż staną się jednoznaczne.

## Adresy zdalne {#remote-addresses}

| Adres | Transport |
|---|---|
| `https://host/path`, `http://host/path` | smart HTTP |
| `ssh://user@host/path`, `user@host:path` | SSH |
| `file:///path`, `/path`, `../path` | repozytorium lokalne, obsługiwane w procesie |

`url.<base>.insteadOf` przepisuje adresy według najdłuższego pasującego
przedrostka; przy wypychaniu pierwszeństwo ma `url.<base>.pushInsteadOf`.
Adresy `git://` i `ext::` kończą się błędem `E_GIT_UNSUPPORTED`.

## Blokady {#locks}

Plik katalogu repozytorium nie jest zmieniany, dopóki istnieje jego plik
blokady `<file>.lock`, co oznacza, że proces `git` go zmienia albo zakończył
się bez usunięcia blokady. Polecenie kończy się błędem `E_GIT_LOCKED` i
proponuje usunięcie blokady jako poprawkę.

## Błędy {#errors}

| Kod | Problem | Przyczyna |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `not_repository`, `path_not_found`, `unsafe_repository` | ścieżka nie jest w repozytorium, nie istnieje albo należy do innego konta |
| `E_GIT_UNSUPPORTED` | `unsupported_feature`, `unsupported_transport` | rozszerzenie repozytorium lub schemat adresu bez implementacji |
| `E_GIT_REVISION` | `not_found`, `ambiguous`, `invalid_syntax`, `not_a_commit` | wskazanie rewizji, którego nie można rozstrzygnąć |
| `E_GIT_LOCKED` | `lock_exists` | obecny plik blokady `git` |
| `E_INVALID_VALUE` | `outside_repository`, `invalid_url` | ścieżka poza drzewem roboczym, adres, którego nie można przeanalizować |
| `E_EXTERNAL_FAILED` | `filter_failed` | wymagany sterownik filtra zawiódł |
| `E_STALE` | | plik zmienił się między odczytem a zapisem |
| `E_IO` | `corrupt_repository`, `repository_error`, `filter_failed` | dane repozytorium, których nie można odczytać ani skonwertować |

## Polecenia {#commands}

| Polecenie | Przeznaczenie |
|---|---|
| [`git.status`](../commands/git.status.md) | stan drzewa roboczego i indeksu |
| [`git.diff`](../commands/git.diff.md) | różnice z identyfikatorami fragmentów |
| [`git.log`](../commands/git.log.md) | historia |
| [`git.show`](../commands/git.show.md) | rewizja, znacznik lub plik w danej rewizji |
| [`git.blame`](../commands/git.blame.md) | pochodzenie wierszy |
| [`git.branch`](../commands/git.branch.md) | gałęzie |
| [`git.tag`](../commands/git.tag.md) | znaczniki |
| [`git.stage`](../commands/git.stage.md) | indeks: pliki, fragmenty i wiersze |
| [`git.commit`](../commands/git.commit.md) | rewizje |
| [`git.review`](../commands/git.review.md) | kontrole zmian przed rewizją |
| [`git.checkpoint`](../commands/git.checkpoint.md) | migawki drzewa roboczego |
| [`git.worktree`](../commands/git.worktree.md) | powiązane drzewa robocze |
| [`git.clone`](../commands/git.clone.md) | kopia repozytorium |
| [`git.fetch`](../commands/git.fetch.md) | pobranie ze zdalnego repozytorium |
| [`git.switch`](../commands/git.switch.md) | zmiana bieżącej gałęzi |
| [`git.restore`](../commands/git.restore.md) | przywrócenie plików z rewizji lub indeksu |
| [`git.reset`](../commands/git.reset.md) | przesunięcie bieżącej gałęzi |
| [`git.stash`](../commands/git.stash.md) | odłożenie zmian na bok |
| [`git.merge`](../commands/git.merge.md) | scalenie historii |
| [`git.rebase`](../commands/git.rebase.md) | odtworzenie rewizji |
| [`git.push`](../commands/git.push.md) | wysłanie do zdalnego repozytorium |
| [`git.pull`](../commands/git.pull.md) | pobranie i zintegrowanie |

## Przykłady {#examples}

Stan repozytorium, które zawiera podkatalog:

```nurt
git.status "/work/app/src/net"
```

Poprzednia pozycja gałęzi z jej reflogu:

```nurt
git.log "/work/app" revision: "main@{1}" count: 1
```

Dodanie katalogu do indeksu z pominięciem plików blokad:

```nurt
git.stage "/work/app" action: add paths: ["/work/app/docs"] exclude: ["*.lock"]
```

Historia gałęzi nadrzędnej bieżącej gałęzi:

```nurt
git.log "/work/app" revision: "@{upstream}" count: 5
```
