---
name: text-editing
description: Odczyt, wyszukiwanie, zamiana, wstawianie i usuwanie tekstu w plikach.
commands: [read, lines, search, replace, insert, edit, remove, write, assert]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/text-editing.md -->
<!-- Responsibility:   Documents text-editing. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# edycja tekstu {#text-editing}

## Odczyt zakresów {#reading-ranges}

`read` z parametrem `lines` zwraca zakres wierszy, `last` zwraca koniec pliku, a `around` z `context` zwraca wiersze wokół pozycji. `lines` tworzy jeden rekord `line` na wiersz, więc kolejne etapy mogą je filtrować.

```nurt
read "/srv/app/main.go" lines: 10..40
lines "/srv/app/main.go" |> filter text ~ /^func /
```

## Wyszukiwanie {#searching}

`search` przyjmuje wzorzec i ścieżkę. `mode` wybiera `literal`, `regex`, `syntax` lub `auto`, `case_sensitive` i `whole_word` doprecyzowują dopasowanie, `context`, `before` i `after` dodają otaczające wiersze, a `output` zwraca dopasowania, nazwy plików, liczby albo pliki bez dopasowania.

```nurt
search "deprecated" "/srv/app/src" glob: "**/*.go" context: 2
search /oldName\(/ "/srv/app/src" glob: "**/*.go" output: files
```

## Zamiana {#replacing}

`replace` zmienia tekst w plikach wyszukiwania lub w plikach wskazanych przez `in`. `occurrence` ogranicza zmianę do jednego wystąpienia, `whitespace` wybiera sposób porównywania białych znaków, a `template` włącza grupy `$1`, `$2` w zamienniku. `expect` zatrzymuje zmianę błędem `E_ASSERT`, chyba że liczba zamian jest podaną liczbą.

```nurt
search /oldName\(/ "/srv/app/src" glob: "**/*.go" |> assert paths: 12 |> replace "newName("
replace "colour" "color" in: "/srv/app/docs/a.md" expect: 3
```

## Wstawianie i usuwanie {#inserting-and-removing}

`insert` dodaje tekst przed wierszem, dopasowaniem lub węzłem albo po nim, lub na początku albo końcu pliku. `remove` zastosowane do rekordów `line` lub `match` usuwa je. Oba zachowują kodowanie i zakończenia wierszy pliku.

```nurt
insert "import \"fmt\"\n" in: "/srv/app/main.go" line: 3 position: after
search "debug_print" "/srv/app/src" glob: "**/*.go" |> remove
```

## Kilka zmian jednego pliku {#several-changes-to-one-file}

`edit` stosuje listę zmian do jednego pliku jako pojedynczy zapis: wszystkie zostają zastosowane albo żadna. Zmiana, która nie pasuje, kończy się błędem `E_NO_MATCH`, a plik pozostaje taki, jaki był. `write` z `mode: append` lub `mode: prepend` dodaje tekst bez wcześniejszego odczytu pliku.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
