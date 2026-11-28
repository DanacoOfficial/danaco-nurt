---
command: edit
group: core
summary: Stosuje listę zmian do jednego pliku, wszystkie albo żadną.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# edit

## Składnia {#synopsis}

```text
edit <path> changes: <list<map<any>>> [name: value]...
```

## Opis {#description}

`edit` stosuje listę zmian do jednego pliku tekstowego jako jedną operację:
albo każda zmiana zostaje zastosowana, albo plik nie zostaje zmieniony. Zmiana
jest mapą jednego z trzech rodzajów:

| Zmiana | Klucze | Skutek |
|---|---|---|
| zastąpienie | `replace` (tekst lub wyrażenie regularne), `with` oraz opcjonalnie `occurrence`, `whole_word`, `whitespace`, `on_missing`, `expect` | zastępuje dopasowania wzorca, tak jak `replace` |
| wstawienie | `insert` (tekst) oraz opcjonalnie `line` i `position` | wstawia wiersze przed `line` lub po nim, albo na początku (`start`) lub na końcu (`end`) pliku, tak jak `insert` |
| usunięcie | `delete_lines` (zakres lub numer wiersza) | usuwa wiersze zakresu |

Każda zmiana jest lokalizowana w pliku w postaci sprzed edycji: numery wierszy
i dopasowania zmiany odnoszą się do oryginalnego tekstu, a nigdy do tekstu
pozostawionego przez zmiany wymienione wcześniej. Wstawienia w tym samym
miejscu zachowują kolejność listy, a wstawienie w wierszu, w którym zaczyna się
usunięcie, poprzedza to usunięcie, więc `{delete_lines: a..b}` razem z
`{insert: …, line: a, position: before}` zastępuje wiersze od `a` do `b`.
Zmiany o nakładających się zakresach są odrzucane, zanim cokolwiek zostanie
zapisane.

`whitespace` i `on_missing` polecenia dotyczą zastąpień bez własnej wartości.
`expect` polecenia dotyczy każdego zastąpienia bez własnego `expect`.
Zastąpienie, które z `on_missing: skip` niczego nie dopasowuje, jest
zgłaszane jako `nothing_matched` wraz ze swoim numerem. Lista zmian, która
pozostawia tekst bez zmian, niczego nie zapisuje i jest zgłaszana jako
`unchanged`.

Plik zachowuje swoje kodowanie, znacznik kolejności bajtów i zakończenia
wierszy; wstawione i zastąpione wiersze przyjmują zakończenia wierszy pliku.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik do zmiany. |
| `changes` | `list<map<any>>` | wymagany | Zmiany: replace i with, insert z line i position albo delete_lines. |
| `expect` | `range` | brak | Oczekiwana liczba zastąpień każdej zmiany. |
| `if_match` | `text` | brak | Oczekiwany etag pliku. |
| `whitespace` | `enum(exact\|trailing\|indent\|normalize)` | `normalize` | Tolerancja różnic w białych znakach. |
| `on_missing` | `enum(skip\|error)` | `skip` | Zachowanie, gdy zmiana niczego nie dopasowuje. |
| `validate` | `enum(off\|warn\|reject)` | `reject` | Wycofuje zmiany, które dodają nowy błąd składni do znanego formatu. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Zmiana wartości i dodanie wiersza po niej:

```nurt example
edit "/home/user/app/config.toml" changes: [{replace: "debug = true", with: "debug = false"}, {insert: "log = \"info\"", line: 2, position: after}]
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/config.toml", "type": "file", "content_utf8": "[server]\ndebug = true\nport = 8080\n" }
] }
```

Zastąpienie wierszy od 2 do 4 jednym wierszem:

```nurt example
edit "/home/user/app/util.py" changes: [{delete_lines: 2..4}, {insert: "    return a + b", line: 2, position: before}]
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/util.py", "type": "file", "content_utf8": "def add(a, b):\n    total = a + b\n    print(total)\n    return total\n" }
] }
```

Podniesienie wersji poprawki, gdy istnieje dokładnie jeden wiersz wersji:

```nurt example
edit "/home/user/app/Cargo.toml" changes: [{replace: /version = "(\d+)\.(\d+)\.\d+"/, with: "version = \"$1.${2}.3\"", occurrence: 1}] expect: 1
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/app/Cargo.toml", "type": "file", "content_utf8": "version = \"1.4.2\"\nedition = \"2024\"\n" }
] }
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | zmiana nie ma rodzaju albo ma ich kilka, ma nieznany klucz lub nakłada się na inną zmianę | popraw zmianę albo połącz nakładające się zmiany |
| `E_OUT_OF_RANGE` | wiersz leży poza plikiem albo zastąpienie z `on_missing: error` wskazuje wystąpienie powyżej liczby swoich dopasowań | podaj wiersz od 1 do liczby wierszy albo mniejsze `occurrence` |
| `E_MISSING_PARAM` | zastąpienie nie ma `with` albo wstawienie przed wierszem lub po nim nie ma `line` | dodaj klucz albo użyj `position: start` lub `end` |
| `E_ASSERT` | liczba dopasowań zastąpienia jest poza `expect` | rozszerz wzorzec, ustaw `occurrence` albo zmień `expect` |
| `E_NO_MATCH` | zastąpienie niczego nie dopasowuje, a `on_missing` ma wartość `error` | odczytaj bieżący tekst i podaj istniejący fragment |
| `E_REGEX` | wyrażenie regularne lub szablon jest nieprawidłowe | popraw wzorzec albo szablon |
| `E_STALE` | plik zmienił się od odczytu albo nie ma znacznika `if_match` | odczytaj plik ponownie i powtórz edycję |
| `E_ENCODING` | kodowanie pliku nie może przedstawić nowego tekstu | najpierw przekonwertuj plik poleceniem `convert` |
| `E_BINARY` | plik jest binarny | edytuj tylko pliki tekstowe |
| `E_ROLLED_BACK` | edycja dodała błąd składni, a `validate` ma wartość `reject` | popraw zmiany albo przekaż `validate: warn` |

## Powiązane {#related}

`replace`, `insert`, `write`, `patch`, `read`.
