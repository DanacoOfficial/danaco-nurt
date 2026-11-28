---
command: git.show
group: git
summary: Pokazuje rewizje wraz z ich zmianami albo zawartość pliku w danej rewizji.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.show

## Składnia {#synopsis}

```text
git.show <repo> <revision> [name: value]...
```

## Opis {#description}

Pokazuje każdy obiekt wskazany przez `revision` i `revs`, w tej kolejności.
Rewizja jest pokazywana jako rekord `commit`, po którym następuje jeden rekord
`hunk` dla każdej zmiany względem jej pierwszego rodzica, z wykrywaniem
zmienionych nazw plików tak, jak wykrywa je `git show`; rewizja początkowa jest
porównywana z pustym drzewem.

Scalenie jest pokazywane ze skróconą różnicą łączoną z `git show` (`--cc`):
brane są pod uwagę tylko pliki różniące się od każdego rodzica, każdy wiersz
ma po jednej kolumnie prefiksu na rodzica, a grupa zmian jest pomijana, gdy
rodzice zawierają jej najwyżej dwie wersje i scalenie przyjmuje jedną z nich bez
zmian. Scalenie, które tylko przejmuje zmiany od rodziców, nie pokazuje więc
żadnych fragmentów. Pola `old_start` i `old_len` fragmentu łączonego podają
wiersze pierwszego rodzica.

Opisany znacznik jest pokazywany jako rekord `git_tag`, po którym następuje
obiekt, który znacznik wskazuje; znaczniki znaczników są śledzone do końca.
Rewizja, która wskazuje drzewo lub plik, jest odrzucana; zawartość pliku
odczytuje się przez `file`.

Przy `file` zawartość pliku w rewizji `revision` jest zwracana jako jeden rekord
`content`. `offset` i `limit` wybierają wiersze; zawartość jest odczytywana tak,
jak jest przechowywana w repozytorium, bez filtrów konwersji. Zawartość binarna
jest raportowana bez tekstu.

Zmiana bez różnic w wierszach, taka jak zmiana trybu lub dokładna zmiana nazwy,
jest jednym fragmentem bez wierszy. Zmiana binarna jest jednym fragmentem z
`binary: true` i bez wierszy.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `revision` | `text` | wymagany | Rewizja do pokazania. |
| `revs` | `list<text>` | brak | Dalsze rewizje do pokazania. |
| `file` | `path` | brak | Zwraca zawartość tego pliku w danej rewizji. |
| `paths` | `list<path>` | brak | Tylko zmiany w tych plikach lub katalogach. |
| `offset` | `int` | brak | Pierwszy zwracany wiersz zawartości pliku, liczony od 1. |
| `limit` | `int` | brak | Maksymalna liczba zwracanych wierszy zawartości pliku. |
| `stat` | `bool` | `false` | Tylko zmienione pliki z liczbą dodanych i usuniętych wierszy. |
| `context` | `int` | `3` | Liczba wierszy kontekstu wokół każdej zmiany. |
| `exclude` | `list<glob>` | `[]` | Ścieżki do pominięcia; raport zlicza pominięte pliki. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `commit`, `hunk`, `git_tag`, `content` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |
| External use | `user_program`: `gpg`; Uruchamia gpg (gpg.openpgp.program lub gpg.program), aby zweryfikować podpisy OpenPGP, gdy wybrano pole signature rewizji lub znaczników. |

## Przykłady {#examples}

Pokaż ostatnią rewizję wraz z jej zmianami.

```nurt example
git.show "/home/user/repo" "HEAD"
```

```text expected
commit short_id: 4f2a9c1 subject: "fix(parser): accept empty input"
hunk path: src/parser.rs old_start: 41 old_len: 6 new_start: 41 new_len: 8 header: "fn parse(input: &str) -> Result<Ast> {"
```

Pokaż pliki zmienione przez scalenie względem jego pierwszego rodzica.

```nurt example
git.show "/home/user/repo" "HEAD" stat: true
```

Pokaż znacznik wydania i rewizję, którą wskazuje, bez plików blokad.

```nurt example
git.show "/home/user/repo" "v1.4.0" exclude: ["*.lock"]
```

Odczytaj wiersze od 100 do 149 pliku w stanie sprzed dwóch rewizji.

```nurt example
git.show "/home/user/repo" "HEAD~2" file: "/home/user/repo/src/main.rs" offset: 100 limit: 50
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `repo` nie znajduje się w repozytorium albo ścieżka leży poza nim (problem `path_outside_repo`). | Podaj ścieżkę wewnątrz drzewa roboczego |
| `E_GIT_REVISION` | Rewizja jest nieznana, niejednoznaczna, jest zakresem (`range`) albo wskazuje drzewo lub plik (`not_a_commit`). | Podaj rewizję lub znacznik; pliki odczytuj przez `file` |
| `E_NOT_FOUND` | `file` nie istnieje w danej rewizji. | Znajdź rewizje pliku poleceniem `git.log` |
| `E_INVALID_VALUE` | `file` wskazuje katalog lub podmoduł, `file` z `revs`, `paths` lub `stat`, `offset` lub `limit` bez `file` albo `offset: 0`. | Usuń kolidujący argument |
| `E_TYPE` | `exclude` nie jest listą wzorców. | Podaj wzorce glob |
| `E_IO` | Nie można odczytać obiektu repozytorium. | Sprawdź repozytorium poleceniem `git.status` |

## Powiązane {#related}

`git.log`, `git.diff`, `git.blame`, `git.tag`.
