---
command: git.log
group: git
summary: Wyświetla rewizje danej wersji, opcjonalnie ograniczone do plików, autorów, dat lub zawartości.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.log

## Składnia {#synopsis}

```text
git.log <repo> [name: value]...
```

## Opis {#description}

Wyświetla rewizje w domyślnej kolejności `git log`: od najnowszej według czasu
wprowadzającego rewizję (committer), bez sortowania topologicznego; rewizje o
równych czasach pojawiają się w kolejności, w jakiej je osiągnięto. `revision`
przyjmuje pojedynczą wersję, zakres `A..B` (rewizje osiągalne z `B` i
nieosiągalne z `A`), zakres symetryczny `A...B`, wykluczenie `^A`, postaci
`A^@` i `A^!` oraz kilka z nich oddzielonych spacjami. Repozytorium bez rewizji
ma pustą historię.

Z `path` historia jest upraszczana tak, jak upraszcza ją `git log -- <path>`:
rewizja jest wyświetlana, gdy zmienia jedną ze ścieżek, a scalenie równe jednemu
ze swoich rodziców w obrębie tych ścieżek jest śledzone tylko przez tego
rodzica. Ścieżka wskazuje plik albo katalog wraz ze wszystkim poniżej.

`follow` wyświetla historię jednego pliku przez zmiany nazw i kopie: gdy plik
pojawia się w rewizji i ma podobne źródło w rodzicu, źródło jest śledzone od
tego miejsca. `symbol` wyświetla rewizje, które zmieniły wiersze jednego
elementu kodu pliku, tak jak robi to `git log -L`; wiersze są znajdowane za
pomocą usługi kodu w pierwszej wyświetlanej wersji i przenoszone do starszych
rewizji przez różnice.

`pickaxe` z tekstem zachowuje rewizje, które zmieniają liczbę wystąpień tekstu
w pliku (`git log -S`); z wyrażeniem regularnym zachowuje rewizje, które
zmieniają liczbę dopasowań (`-S --pickaxe-regex`). `diff_regex` zachowuje
rewizje z dodanym lub usuniętym wierszem, który pasuje (`git log -G`). Obu nie
można łączyć. Scalenia nie mają własnych zmian, więc `follow`, `pickaxe` i
`diff_regex` nigdy ich nie wybierają; pliki binarne i podmoduły nie są
przeszukiwane.

`author` to wyrażenie regularne dopasowywane do `Name <email>`, a `message` do
każdego wiersza komunikatu. Tożsamości są mapowane przez `.mailmap`. Pole
`trailers` rekordu rewizji mapuje każdy token stopki na jego wartość; token,
który występuje kilka razy, zachowuje każdą wartość, po jednej w wierszu w
kolejności komunikatu, tak jak wyświetla je `git interpret-trailers`, w
`git.log` i w każdym innym poleceniu zwracającym rekordy rewizji. `since`
zatrzymuje przechodzenie na pierwszej rewizji starszej niż chwila; `until`
ukrywa rewizje od niej nowsze. Oba porównują czas wprowadzającego rewizję w
pełnych sekundach i obejmują granicę.

`group_by` zastępuje rewizje rekordami `map` z liczbą wybranych rewizji według
typu Conventional Commits (`other` dla innych tematów), według autora lub
według dnia autora, od najczęstszych.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `count` | `int` | `20` | Maksymalna liczba rewizji. |
| `skip` | `int` | `0` | Liczba rewizji do pominięcia. |
| `path` | `list<path>` | brak | Tylko rewizje, które zmieniają te pliki lub katalogi. |
| `revision` | `text` | `"HEAD"` | Wersja lub zakres do wyświetlenia. |
| `author` | `text` | brak | Tylko rewizje, których autor pasuje do tego tekstu. |
| `since` | `instant` | brak | Tylko rewizje z tej chwili lub późniejsze. |
| `until` | `instant` | brak | Tylko rewizje sprzed tej chwili. |
| `message` | `regex` | brak | Tylko rewizje, których komunikat pasuje do tego wzorca. |
| `pickaxe` | `text \| regex` | brak | Tylko rewizje, które dodają lub usuwają ten tekst lub wzorzec. |
| `follow` | `bool` | `false` | Śledzi pojedynczy plik przez zmiany nazw. |
| `symbol` | `text` | brak | Tylko rewizje, które zmieniają wiersze tego elementu kodu. |
| `stat` | `bool` | `false` | Dołącza zmienione pliki z liczbą dodanych i usuniętych wierszy. |
| `group_by` | `enum(type\|author\|day)` | brak | Grupuje rewizje według typu conventional commit, autora lub dnia. |
| `diff_regex` | `regex` | brak | Tylko rewizje, których zmienione wiersze pasują do tego wzorca. |
| `exclude` | `list<glob>` | `[]` | Ścieżki do pominięcia, także w filtrach pickaxe i diff_regex. |
| `find_renames` | `bool` | `false` | Wykrywa pliki o zmienionej nazwie przy ograniczaniu dziennika do ścieżek. |
| `find_copies` | `bool` | `false` | Wykrywa skopiowane pliki przy ograniczaniu dziennika do ścieżek. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `commit`, `group` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |
| External use | `user_program`: `gpg`; Uruchamia gpg (gpg.openpgp.program lub gpg.program), aby zweryfikować podpisy OpenPGP, gdy wybrano pole signature rewizji. |

## Przykłady {#examples}

Wyświetlenie ostatnich pięciu rewizji.

```nurt example
git.log "/home/user/repo" count: 5
```

```text expected
commit short_id: 4f2a9c1 subject: "fix(parser): accept empty input" author: "Ann Lee"
commit short_id: 9b07d3e subject: "feat: add the export command" author: "Ann Lee"
```

Wyświetlenie rewizji gałęzi, których nie ma w `main`, ze zmienionymi plikami.

```nurt example
git.log "/home/user/repo" revision: "main..topic" stat: true
```

Śledzenie jednego pliku przez zmiany nazw i zachowanie rewizji, które dodały lub
usunęły wywołanie.

```nurt example
git.log "/home/user/repo" path: "/home/user/repo/src/io.rs" follow: true pickaxe: "read_all("
```

Zliczenie rewizji z ostatnich 30 dni według typu Conventional Commits.

```nurt example
git.log "/home/user/repo" since: ago(30d) count: 1000 group_by: type
```

Wyświetlenie historii jednej funkcji.

```nurt example
git.log "/home/user/repo" path: "/home/user/repo/src/lib.rs" symbol: "parse_header"
```

Odpowiedź, która pomija rekordy, kończy się wskazówką „more: next” i zawiera
pole `continuation`; polecenie `next` zwraca następną część bez ponownego
uruchamiania polecenia.

```text
matched 900  shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `repo` nie znajduje się w repozytorium albo ścieżka leży poza nim (problem `path_outside_repo`). | Podaj ścieżkę wewnątrz drzewa roboczego |
| `E_GIT_REVISION` | Wersja jest nieznana (`not_found`), niejednoznaczna (`ambiguous`), niepoprawnie zapisana (`invalid_syntax`) albo nie wskazuje rewizji (`not_a_commit`). | Wyświetl gałęzie poleceniem `git.branch` |
| `E_REGEX` | `author`, `message`, `pickaxe` lub `diff_regex` nie jest poprawnym wzorcem. | Popraw wzorzec |
| `E_MISSING_PARAM` | `follow` lub `symbol` bez pliku w `path`. | Podaj jeden plik w `path` |
| `E_INVALID_VALUE` | Więcej niż jeden plik z `follow` lub `symbol`, `symbol` z `follow` lub `stat`, `pickaxe` z `diff_regex`, puste `pickaxe`, ujemne `count` lub `skip` albo nieznane `group_by`. | Usuń jeden z argumentów |
| `E_UNSUPPORTED` | `symbol` na hoście bez usługi kodu. | Użyj `path` bez `symbol` |
| `E_UNSUPPORTED_LANGUAGE` | `symbol` w pliku bez gramatyki. | Użyj `path` bez `symbol` |
| `E_SYMBOL_NOT_FOUND` | Element nie jest zdefiniowany albo jest zdefiniowany kilka razy; kandydaci są wymienieni. | Użyj nazwy kwalifikowanej spośród kandydatów |
| `E_SYMBOL_AMBIGUOUS` | Element nie jest zdefiniowany albo jest zdefiniowany kilka razy; kandydaci są wymienieni. | Użyj nazwy kwalifikowanej spośród kandydatów |
| `E_NOT_FOUND` | Plik z `symbol` nie istnieje w wyświetlanej wersji. | Podaj istniejący plik |
| `E_IO` | Nie można odczytać obiektu repozytorium. | Sprawdź repozytorium poleceniem `git.status` |

## Powiązane {#related}

`git.show`, `git.blame`, `git.diff`, `git.branch`, `git.tag`.
