---
name: encodings
description: Wykrywanie, zachowywanie i konwersja kodowań tekstu oraz zakończeń wierszy.
commands: [info, read, write, convert, find, filter, select]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/encodings.md -->
<!-- Responsibility:   Documents encodings. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# kodowania {#encodings}

## Wykrywanie {#detection}

Pliki tekstowe są odczytywane w kodowaniu, które Nurt wykrywa: na podstawie znacznika kolejności bajtów, poprawnego UTF-8 lub strony kodowej. Rekord `entry` polecenia `info` i `find` zawiera pola `encoding`, `line_ending` i `binary`, więc zbiór plików można sprawdzić przed jakąkolwiek zmianą.

```nurt
find "/srv/app" glob: "**/*.txt" |> select path encoding line_ending
```

## Zachowywanie {#preservation}

Zmiana pliku tekstowego zachowuje jego kodowanie, zakończenia wierszy i znacznik kolejności bajtów. Wynik poleceń jest zawsze w UTF-8. `read` z parametrem `encoding` wymusza kodowanie, gdy wykrywanie jest błędne, a `write` z parametrami `encoding`, `line_ending` i `bom` ustawia je dla nowego pliku.

```nurt
read "/srv/legacy/readme.txt" encoding: "windows-1250"
write "/srv/app/notes.txt" "first\nsecond\n" mode: create line_ending: crlf bom: true
```

## Konwersja {#conversion}

`convert` zmienia kodowanie, zakończenia wierszy lub znacznik kolejności bajtów plików. Parametr, który nie został podany, pozostawia daną właściwość bez zmian, a `line_ending: keep` stwierdza to jawnie. Konwersja jest rejestrowana w dzienniku i można ją cofnąć.

```nurt
convert "/srv/legacy/readme.txt" encoding: "utf-8" line_ending: lf
find "/srv/app" glob: "**/*.txt" |> filter line_ending == "crlf" |> convert line_ending: lf
```

## Zawartość mieszana i niepoprawna {#mixed-and-invalid-content}

Plik o mieszanych zakończeniach wierszy zachowuje każde zakończenie, dopóki nie zostanie zastosowane `convert`. Sekwencja bajtów niepoprawna w danym kodowaniu kończy się błędem `E_ENCODING` z pozycją pierwszego niepoprawnego bajtu. Plik zawierający dane binarne jest pomijany przez `search` i `replace`, chyba że podano `binary: text`.

## Błędy {#errors}

`E_ENCODING` zgłasza niepoprawną sekwencję lub nieobsługiwaną nazwę kodowania. `E_BINARY` zgłasza, że polecenie tekstowe napotkało zawartość binarną. `E_UNSUPPORTED` zgłasza kodowanie, którego Nurt nie konwertuje.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
