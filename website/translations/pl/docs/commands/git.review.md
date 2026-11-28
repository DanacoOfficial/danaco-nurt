---
command: git.review
group: git
summary: Sprawdza zmiany przed rewizją pod kątem znaczników konfliktu, błędów białych znaków, dużych lub binarnych plików, sekretów, brakujących testów i formatu tematu.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.review

## Składnia {#synopsis}

```text
git.review <repo> [name: value]...
```

## Opis {#description}

Sprawdza zmiany przed rewizją i zgłasza jeden rekord `finding` dla każdego
problemu. Repozytorium nie jest zmieniane.

Bez `base` zmiany wybiera `scope`:

- `staged`: indeks względem `HEAD` (względem pustego drzewa przed pierwszą
  rewizją);
- `working`: drzewo robocze względem indeksu, pliki z konfliktami względem
  etapu `ours` oraz nieśledzone pliki, które nie są ignorowane;
- `all`: drzewo robocze względem `HEAD`, z nieśledzonymi plikami, które nie są
  ignorowane.

Przy `base` pliki z `target` (wartość domyślna `HEAD`) są porównywane z plikami
z `base`, a `scope` nie ma wpływu. Usunięte pliki, dowiązania i podmoduły nie
są sprawdzane. Odczytywanych jest najwyżej 300 nieśledzonych plików;
nieśledzony plik większy niż 512 KiB jest traktowany jako binarny bez
odczytywania jego zawartości.

Kontrole dotyczą dodanych wierszy różnicy z trzema wierszami kontekstu, czyli
różnicy, którą pokazuje `git.diff` i której identyfikatory fragmentów
akceptują `git.stage` oraz `git.restore`:

| Kontrola | Reguła | Waga | Ustalenie |
|---|---|---|---|
| `secrets` | identyfikator reguły | reguły | Dodany wiersz pasuje do reguły usługi reguł przeglądu. Wartość nigdy nie jest raportowana. |
| `conflict_markers` | `conflict_markers` | błąd | Dodany wiersz zaczyna się od `<<<<<<< ` lub `>>>>>>> `; jedno ustalenie na wiersz. |
| `whitespace` | `whitespace` | ostrzeżenie | Dodane wiersze kończą się spacjami lub tabulatorami; jedno ustalenie na plik z liczbą wystąpień. Pliki Markdown (`.md`, `.markdown`) są wyłączone z tej reguły. |
| `whitespace` | `space_before_tab` | ostrzeżenie | Wcięcie dodanych wierszy zawiera spację przed tabulatorem; jedno ustalenie na plik. |
| `whitespace` | `blank_at_eof` | ostrzeżenie | Zmiana dodaje puste wiersze na końcu pliku; wskazany wiersz jest pierwszym z nich. |
| `debug_statements` | `debug_statements` | ostrzeżenie | Dodany wiersz pliku źródłowego wywołuje `console.log(`, `console.debug(`, `debugger`, `dbg!(`, `pdb.set_trace(` lub `breakpoint()`. |
| `final_newline` | `final_newline` | ostrzeżenie | Zmiana pozostawia plik bez końcowego znaku nowego wiersza. |
| `large_files` | `large_files` | ostrzeżenie | Nowa zawartość jest większa niż 512 KiB; `value` i `threshold` podają rozmiary w bajtach. |
| `binary` | `binary` | ostrzeżenie | Nowa zawartość jest binarna (bajt NUL w pierwszych 8000 bajtach albo atrybut `binary`); kontrole wierszy nie są stosowane. |
| `tests` | `tests_missing`, `tests_unchanged` | ostrzeżenie | Plik źródłowy się zmienił i żadna zmieniona ścieżka testowa nie zawiera jego nazwy; `tests_unchanged` wskazuje istniejący test. |
| `subject` | patrz niżej | patrz niżej | Wiersz tematu z `message` łamie regułę. |

Plikami źródłowymi są pliki z rozszerzeniami `c`, `cc`, `cjs`, `cpp`, `cs`,
`cxx`, `go`, `h`, `hpp`, `java`, `js`, `jsx`, `kt`, `kts`, `mjs`, `php`,
`py`, `rb`, `rs`, `scala`, `swift`, `ts`, `tsx` i `vue`. Ścieżka testowa leży
w katalogu `test`, `tests`, `spec`, `specs`, `__tests__` lub `testing` albo jej
nazwa ma postać `test_*`, `*_test.*`, `*.test.*`, `*.spec.*`, `*_spec.*`,
`*Test.*` lub `*Tests.*`. Nazwą pliku źródłowego jest jego rdzeń; dla rdzeni
`index`, `main`, `mod`, `lib`, `__init__`, `utils` i `common` jest to nazwa
katalogu.

Reguły `subject` dotyczą pierwszego niepustego wiersza `message`:

| Reguła | Waga | Stosowanie | Ustalenie |
|---|---|---|---|
| `subject_empty` | błąd | zawsze | Komunikat nie ma wiersza tematu. |
| `conventional_header` | błąd | `conventional: true` | Temat nie ma postaci `type(scope)!: description`. |
| `missing_scope` | ostrzeżenie | `conventional: true` | Nagłówek nie ma zakresu. |
| `subject_case` | ostrzeżenie | `conventional: true` | Opis zaczyna się wielką literą. |
| `subject_length` | ostrzeżenie | zawsze | Temat ma więcej niż `subject_max` znaków. |
| `subject_period` | ostrzeżenie | zawsze | Temat kończy się kropką. |
| `subject_mood` | ostrzeżenie | zawsze | Pierwsze słowo opisu jest formą przeszłą lub trzeciej osoby, taką jak `added` lub `fixes`. |
| `blank_line` | ostrzeżenie | zawsze | Drugi wiersz komunikatu nie jest pusty. |

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `scope` | `enum(all\|staged\|working)` | `all` | Zmiany w indeksie, zmiany w drzewie roboczym albo oba rodzaje. |
| `paths` | `list<path>` | brak | Tylko zmiany w tych plikach lub katalogach. |
| `base` | `text` | brak | Rewizja bazowa zakresu przeglądanego zamiast niezatwierdzonych zmian. |
| `target` | `text` | brak | Rewizja końcowa zakresu. |
| `message` | `text` | brak | Planowany komunikat rewizji do sprawdzenia. |
| `checks` | `list<enum(conflict_markers\|whitespace\|large_files\|binary\|secrets\|subject\|tests\|debug_statements\|final_newline)>` | brak | Kontrole do uruchomienia; wszystkie, gdy pominięto. |
| `subject_max` | `int` | `72` | Najdłuższy dozwolony wiersz tematu komunikatu rewizji, co najmniej 20. |
| `conventional` | `bool` | `false` | Wymaga wiersza tematu zgodnego z Conventional Commits. |
| `exclude` | `list<glob>` | `[]` | Ścieżki do pominięcia; raport zlicza pominięte pliki. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `finding` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Sprawdź zmiany w indeksie i planowany komunikat przed rewizją.

```nurt example
git.review "/home/user/repo" scope: staged message: "feat(parser): add numbers" conventional: true
```

```text expected
finding path: src/parse.rs kind: conflict_markers severity: error rule: conflict_markers line: 42 hunk: "src/parse.rs#2:9c41e0"
finding path: src/parse.rs kind: whitespace severity: warning rule: whitespace line: 17 count: 3
```

Sprawdź tylko białe znaki w zmianach drzewa roboczego pod `src`.

```nurt example
git.review "/home/user/repo" scope: working paths: ["/home/user/repo/src"] checks: [whitespace, final_newline]
```

Przejrzyj rewizje gałęzi względem `main`, bez plików generowanych.

```nurt example
git.review "/home/user/repo" base: "main" target: "topic" exclude: ["*.lock", "dist/**"]
```

Sprawdź komunikat rewizji z krótszym limitem tematu.

```nurt example
git.review "/home/user/repo" checks: [subject] message: "Fixed the parser." subject_max: 50
```

```text expected
finding path: /home/user/repo target: message kind: subject severity: warning rule: subject_period line: 1
finding path: /home/user/repo target: message kind: subject severity: warning rule: subject_mood line: 1
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | `target` bez `base` (problem `missing_base`). | Podaj `base` |
| `E_INVALID_VALUE` | Nieznany zakres lub kontrola albo `subject_max` poniżej 20 (problem `out_of_range`). | Popraw wartość |
| `E_GIT_NOT_REPOSITORY` | `repo` nie znajduje się w repozytorium albo ścieżka leży poza nim. | Podaj ścieżkę wewnątrz repozytorium |
| `E_GIT_REVISION` | `base` lub `target` nie wskazuje żadnej rewizji. | Użyj gałęzi, znacznika lub rewizji |
| `E_GIT_UNSUPPORTED` | `scope` `all` lub `working` w repozytorium typu bare (problem `bare_repository`). | Użyj `scope: staged` albo zakresu z `base` |
| `E_CANCELLED` | Wywołanie zostało anulowane. | Powtórz wywołanie |

## Powiązane {#related}

`git.commit`, `git.diff`, `git.stage`, `git.restore`, `git.status`.
