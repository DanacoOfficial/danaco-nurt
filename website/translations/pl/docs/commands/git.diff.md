---
command: git.diff
group: git
summary: Pokazuje różnice między wersjami, indeksem i drzewem roboczym.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# git.diff

## Składnia {#synopsis}

```text
git.diff <repo> [name: value]...
```

## Opis {#description}

Porównuje dwie strony repozytorium i zwraca różnice jako fragmenty (hunk) w
formacie unified polecenia `git diff`. Strony są wybierane w następujący sposób.

| Parametry | Strona stara | Strona nowa |
|---|---|---|
| brak | indeks | drzewo robocze |
| `staged` | `HEAD` | indeks |
| `from` | wersja `from` | drzewo robocze |
| `from`, `staged` | wersja `from` | indeks |
| `from`, `to` | wersja `from` | wersja `to` |

`path` wybiera pliki i katalogi dosłownie, względem katalogu głównego
repozytorium. Pliki z etapami konfliktu są pomijane. Zmiany nazw i kopie są
wykrywane zgodnie z `diff.renames` (domyślnie włączone) z progiem podobieństwa
50 procent.

Wiersze są porównywane algorytmem ustawionym w `diff.algorithm` (domyślnie
`myers`, a także `minimal`, `histogram`; `patience` jest obliczany za pomocą
`histogram` i odnotowywany jako `semantic_fallback`) oraz heurystyką wcięć
(`diff.indentHeuristic`, domyślnie włączona). Zmiany oddalone o najwyżej
dwukrotność kontekstu, powiększoną o `diff.interHunkContext`, dzielą jeden
fragment; żądanie układu kanonicznego (bez `from` i `to`, z domyślnym
kontekstem 3 wierszy, bez `function_context`, `ignore_whitespace` i `symbol`)
ignoruje `diff.interHunkContext`, więc jego fragmenty i ich identyfikatory nie
zależą od konfiguracji. Tekst po `@@` to najbliższy wiersz funkcji powyżej
fragmentu, znaleziony według wzorca sterownika `diff` dla ścieżki
(`diff.<driver>.xfuncname`, wbudowane wzorce dla `rust`, `python`, `golang`,
`java`, `cpp`, `markdown` i `bash` albo domyślna reguła `git`).

Plik jest binarny, gdy ustawiono atrybut `-diff` lub `binary`, gdy jego
sterownik ustawia `diff.<driver>.binary` albo gdy jego pierwsze 8000 bajtów
zawiera bajt NUL; plik binarny daje jeden rekord bez wierszy. Sama zmiana
trybu albo zmiana nazwy bez zmiany zawartości daje jeden rekord bez wierszy.
Brak znaku nowego wiersza na końcu pliku jest zapisywany jako wiersz
`\ No newline at end of file`.

Programy konwersji tekstu (`diff.<driver>.textconv`) i sterowniki filtrów nie
są uruchamiane: porównywana jest zapisana zawartość, a raport odnotowuje
`filter_not_applied`.

### Identyfikatory fragmentów {#hunk-identifiers}

Bez `from`, `to`, `symbol`, `function_context` i `ignore_whitespace` oraz z
domyślnym kontekstem 3 wierszy każdy rekord pliku, którego nazwa nie została
zmieniona, zawiera identyfikator `path#number` i `fingerprint`. Numer zlicza
fragmenty pliku od 1 w różnicy drzewa roboczego względem indeksu albo indeksu
względem `HEAD` z `staged`. Odcisk (fingerprint) to pierwsze sześć cyfr
szesnastkowych SHA-256 wierszy fragmentu, wraz z ich przedrostkami, połączonych
znakami nowego wiersza. `git.stage hunks:` i `git.restore hunks:` przyjmują
identyfikator w postaci `path#number`, `path#number:fingerprint` albo z
wierszami `path#number@2-4,7`; odcisk, który już nie pasuje, jest odrzucany.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `repo` | `path` | wymagany | Repozytorium lub ścieżka wewnątrz niego. |
| `from` | `text` | indeks lub HEAD | Stara wersja. |
| `to` | `text` | drzewo robocze | Nowa wersja. |
| `staged` | `bool` | `false` | Porównuje indeks z HEAD. |
| `path` | `list<path>` | brak | Tylko te pliki lub katalogi. |
| `context` | `int` | `3` | Wiersze kontekstu wokół każdej zmiany. |
| `stat` | `bool` | `false` | Tylko liczby dodanych i usuniętych wierszy dla każdego pliku. |
| `function_context` | `bool` | `false` | Rozszerza każdą zmianę na całą otaczającą funkcję. |
| `symbol` | `text` | brak | Tylko zmiany w obrębie tego elementu kodu po nowej stronie. |
| `word_diff` | `bool` | `false` | Pokazuje zmiany słowo po słowie. |
| `ignore_whitespace` | `bool` | `false` | Ignoruje zmiany w białych znakach. |
| `ignore_moved` | `bool` | `false` | Ukrywa bloki wierszy, które zostały tylko przeniesione. |
| `exclude` | `list<glob>` | `[]` | Ścieżki do pominięcia; raport zlicza wykluczone pliki. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `hunk` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Pokazanie zmian jednego pliku, których nie dodano do poczekalni.

```nurt example
git.diff "/home/user/repo" path: ["/home/user/repo/src/main.rs"]
```

```text expected
hunk path: src/main.rs id: src/main.rs#1 old_start: 10 old_len: 7 new_start: 10 new_len: 8
```

Podsumowanie tego, co gałąź zmieniła od czasu gałęzi głównej.

```nurt example
git.diff "/home/user/repo" from: "main" to: "feature" stat: true
```

Pokazanie zmian w poczekalni z całymi funkcjami, z ignorowaniem białych znaków.

```nurt example
git.diff "/home/user/repo" staged: true function_context: true ignore_whitespace: true
```

Pokazanie zmian jednej funkcji od poprzedniej rewizji.

```nurt example
git.diff "/home/user/repo" from: "HEAD~1" path: ["/home/user/repo/src/parser.rs"] symbol: "parse_expr"
```

Odpowiedź, która pomija rekordy, kończy się wskazówką „more: next” i zawiera
pole `continuation`; polecenie `next` zwraca następną część bez ponownego
uruchamiania polecenia.

```text
matched 120  shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | `to` bez `from` albo `symbol` bez pliku. | Dodaj brakujący parametr |
| `E_ARITY` | `symbol` z więcej niż jednym plikiem. | Podaj jeden plik w `path` |
| `E_ARG` | `staged` razem z `to` (problem `param_conflict`). | Usuń jeden z nich |
| `E_INVALID_VALUE` | `context` poza zakresem od 0 do 200 albo nieprawidłowy wzorzec słowa. | Użyj wartości z zakresu |
| `E_GIT_REVISION` | Wersja nie istnieje albo nie wskazuje rewizji ani drzewa. | Sprawdź nazwę poleceniem `git.log "/home/user/repo"` |
| `E_GIT_NOT_REPOSITORY` | Ścieżka nie znajduje się w repozytorium, ścieżka leży poza nim albo drzewo robocze jest potrzebne w repozytorium typu bare. | Podaj ścieżkę wewnątrz drzewa roboczego |
| `E_SYMBOL_NOT_FOUND` | `symbol` nie jest zdefiniowany w nowej wersji pliku. | Sprawdź nazwę elementu |
| `E_STAGE_FAILED` | Nie można odczytać obiektu, indeksu lub pliku (problem `read_failed`). | Przywróć brakujące obiekty repozytorium |

## Powiązane {#related}

`git.status`, `git.stage`, `git.show`, `git.review`, `git.checkpoint`.
