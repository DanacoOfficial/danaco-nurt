---
command: git.blame
group: git
summary: Pokazuje rewizję i autora, którzy jako ostatni zmienili każdy wiersz pliku.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.blame

## Składnia {#synopsis}

```text
git.blame <path> [name: value]...
```

## Opis {#description}

Opatruje każdy wiersz pliku adnotacją o rewizji, która zmieniła go jako
ostatnia, w kolejności wierszy. Historia jest śledzona tak, jak domyślnie
śledzi ją `git blame`: przez zmiany nazwy całego pliku, z porównywaniem wierszy
algorytmem Myersa. Wiersze przeniesione w obrębie pliku lub skopiowane z innego
pliku są przypisywane rewizji, która je przeniosła lub skopiowała, chyba że
ustawiono `find_moves` lub `find_copies`. Tożsamości są mapowane przez
`.mailmap`. Rewizje początkowe (root) są zgłaszane jako granice.

Bez `revision` adnotacją obejmowany jest plik drzewa roboczego po zastosowaniu
wbudowanych konwersji (`core.autocrlf`, `eol`, `text`); programy filtrujące nie
są uruchamiane. Wiersze, które różnią się od `HEAD`, nie należą do żadnej
rewizji: ich rewizją jest identyfikator pusty (null), a ich autorem
`Not Committed Yet`. Plik, który jest w poczekalni, ale nie ma go w `HEAD`, ma
tylko takie wiersze. Z `revision` adnotacją obejmowany jest plik z tej rewizji.

`find_moves` (`git blame -M`) przypisuje blok wierszy, który rewizja przeniosła
w obrębie pliku, rewizji, która go napisała, gdy blok zawiera co najmniej
20 znaków alfanumerycznych. `find_copies` (`git blame -C`) obejmuje
`find_moves` i dodatkowo szuka bloków o co najmniej 40 znakach
alfanumerycznych w innych plikach, które zmieniła ta sama rewizja. Wiersze
rewizji z wieloma rodzicami są przeszukiwane po kolei w każdym rodzicu.

Podziały wierszy, także znak powrotu karetki przed znakiem nowego wiersza, nie
należą do `text`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik w repozytorium. |
| `lines` | `range` | brak | Tylko te wiersze. |
| `revision` | `text` | `"HEAD"` | Wersja do opatrzenia adnotacją. |
| `find_moves` | `bool` | `false` | Śledzi wiersze przeniesione w obrębie pliku. |
| `find_copies` | `bool` | `false` | Śledzi wiersze skopiowane z innych plików. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `blame_line` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Opatrzenie adnotacją pliku drzewa roboczego.

```nurt example
git.blame "/home/user/repo/src/main.rs"
```

```text expected
blame_line no: 1 short_id: 9b07d3e author: "Ann Lee" text: "use std::io;"
blame_line no: 2 short_id: 0000000 author: "Not Committed Yet" text: "use std::fs;"
```

Opatrzenie adnotacją wierszy od 40 do 60 w postaci z wydania.

```nurt example
git.blame "/home/user/repo/src/parser.rs" lines: 40..60 revision: "v1.4.0"
```

Zliczenie opatrzonych adnotacją wierszy według autora.

```nurt example
git.blame "/home/user/repo/src/parser.rs" revision: "HEAD" |> group author |> sort count order: desc
```

Odpowiedź, która pomija rekordy, kończy się wskazówką „more: next” i zawiera
pole `continuation`; polecenie `next` zwraca następną część bez ponownego
uruchamiania polecenia.

```text
matched 640  shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_GIT_NOT_REPOSITORY` | `path` nie znajduje się w repozytorium. | Podaj plik wewnątrz drzewa roboczego |
| `E_GIT_REVISION` | `revision` jest nieznane, niejednoznaczne albo nie wskazuje rewizji. | Wyświetl gałęzie poleceniem `git.branch` |
| `E_NOT_FOUND` | Plik nie istnieje w `revision` albo nie ma go ani w `HEAD`, ani w poczekalni. | Znajdź plik poleceniem `git.log` albo dodaj go do poczekalni |
| `E_INVALID_VALUE` | `path` wskazuje katalog, podmoduł lub katalog główny repozytorium albo `lines` zaczyna się przed wierszem 1 lub po ostatnim wierszu. | Podaj plik i zakres wewnątrz niego |
| `E_IO` | Nie można odczytać obiektu repozytorium. | Sprawdź repozytorium poleceniem `git.status` |

## Powiązane {#related}

`git.log`, `git.show`, `git.diff`.
