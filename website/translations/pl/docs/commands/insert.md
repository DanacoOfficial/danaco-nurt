---
command: insert
group: core
summary: Wstawia tekst przed lub po wierszach, dopasowaniach albo węzłach, albo na początku lub końcu plików.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# insert

## Składnia {#synopsis}

```text
insert <text> in: <path> [name: value]...
line|match|node|entry |> insert <text> [name: value]...
```

## Opis {#description}

`insert` dodaje wiersze do plików tekstowych. Postać źródłowa wstawia do pliku
`in`: przed lub po `line` z `position: before` lub `after` oraz na początku lub
końcu pliku z `position: start` lub `end`. Bez `line` wartość `before` wstawia
na początku, a `after` na końcu pliku.

Postać etapu wstawia przy każdym rekordzie wejściowym. Rekord `line` lub `match`
wskazuje swój wiersz, a `before` lub `after` umieszcza tekst przed lub po tym
wierszu; `start` i `end` wstawiają raz na początku lub końcu każdego pliku
wskazanego przez rekordy. Rekord `entry` wskazuje cały plik: `before` i `start`
wstawiają na jego początku, `after` i `end` na jego końcu. Kilka rekordów tego
samego wiersza wstawia tekst jeden raz.

Wszystkie wstawienia do jednego pliku są umieszczane względem pliku w postaci
odczytanej: numery wierszy wejścia nie przesuwają się z powodu wcześniejszych
wstawień, a wstawienia w tym samym miejscu zachowują kolejność wejścia.
Wstawiane wiersze przyjmują zakończenia wierszy pliku. Tekst wstawiony na końcu
pliku bez końcowego podziału wiersza pozostawia plik bez niego, chyba że ustawiono
`ensure_newline`. Plik zachowuje swoje kodowanie i znacznik kolejności bajtów.
Puste wejście niczego nie zmienia i jest raportowane jako `nothing_matched`.
Wiersz poza plikiem kończy się błędem `E_OUT_OF_RANGE` i niczego nie zmienia.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `text` | `text` | wymagany | Tekst do wstawienia. |
| `in` | `path` | wymagany | Plik w postaci źródłowej. |
| `line` | `int` | brak | Numer wiersza w postaci źródłowej. |
| `position` | `enum(before\|after\|start\|end)` | `after` | Miejsce tekstu względem wiersza, dopasowania lub pliku. |
| `ensure_newline` | `bool` | `false` | Kończy wstawiany tekst zakończeniem wiersza. |
| `validate` | `enum(off\|warn\|reject)` | `reject` | Wycofuje zmianę, która dodaje nowy błąd składni do znanego formatu. |
| `if_match` | `text` | brak | Oczekiwany etag pliku. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| `source` | `none` | `effect` |
| `stage` | `line`, `match`, `node`, `entry` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wstaw import przed pierwszym wierszem pliku:

```nurt example
insert "use std::fmt;" in: "/home/user/app/src/lib.rs" line: 1 position: before
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/src/lib.rs", "type": "file", "content_utf8": "pub mod parse;\n" }
] }
```

Wstaw komentarz przed wierszem każdego dopasowania wyszukiwania:

```nurt example
search "fn main" "/home/user/app/src/main.rs" |> insert "// Entry point of the program." position: before
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/src/main.rs", "type": "file", "content_utf8": "fn helper() {}\n\nfn main() {\n    helper();\n}\n" }
] }
```

Dopisz wiersz na końcu pliku:

```nurt example
insert "export PATH=\"$HOME/bin:$PATH\"" in: "/home/user/.profile" position: end
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/.profile", "type": "file", "content_utf8": "umask 022\n" }
] }
```

Dodaj komentarz nagłówkowy do każdego wyświetlonego pliku:

```nurt example
find "/home/user/app/src" glob: "*.rs" |> insert "// Part of the parser." position: start
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/src/a.rs", "type": "file", "content_utf8": "fn a() {}\n" },
  { "path": "/home/user/app/src/b.rs", "type": "file", "content_utf8": "fn b() {}\n" }
] }
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_OUT_OF_RANGE` | wiersz leży poza plikiem | podaj wiersz od 1 do liczby wierszy |
| `E_MISSING_PARAM` | postać źródłowa nie ma `in` | wskaż plik parametrem `in` |
| `E_STALE` | plik zmienił się od odczytu albo nie ma znacznika `if_match` | odczytaj plik ponownie i powtórz zmianę |
| `E_BINARY` | plik jest binarny | wstawiaj tylko do plików tekstowych |
| `E_ENCODING` | kodowanie pliku nie może przedstawić tekstu | najpierw przekonwertuj plik poleceniem `convert` |
| `E_ROLLED_BACK` | wstawienie dodało błąd składni, a `validate` ma wartość `reject` | popraw tekst albo przekaż `validate: warn` |

## Powiązane {#related}

`edit`, `replace`, `write`, `lines`, `search`.
