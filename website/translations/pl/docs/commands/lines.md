---
command: lines
group: core
summary: Produkuje wiersze pliku lub zawartości wejścia.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# lines

## Składnia {#synopsis}

```text
lines <path> [<range>]
entry|content |> lines [<range>]
```

## Opis {#description}

`lines` produkuje jeden rekord `line` na każdy wiersz pliku tekstowego. Postać
źródłowa odczytuje wskazany plik; kodowanie jest wykrywane, więc UTF-16 oraz
starsze kodowania, takie jak Windows-1250, są dekodowane. Postać etapu
odczytuje pliki z rekordów wejściowych `entry` oraz tekst z rekordów
wejściowych `content`; wpisy katalogów są pomijane, a pliki binarne są
pomijane i zliczane (uwaga `binary_skipped`).

Wiersz kończy się znakiem przesunięcia wiersza (line feed), który może być
poprzedzony znakiem powrotu karetki; tekst ze znakami powrotu karetki i bez
przesunięcia wiersza jest dzielony na znakach powrotu karetki. Ostatni wiersz
bez zakończenia jest wierszem, a końcowe zakończenie wiersza nie rozpoczyna
kolejnego wiersza, więc pusty plik nie ma wierszy.

`range` wybiera wiersze według numerów liczonych od 1; wiersze za końcem pliku
nie są produkowane. Wiersze rekordu `content` zachowują numery z pliku, więc
`read … lines: 10..20 |> lines` numeruje je od 10.

`start` i `end` są przesunięciami bajtowymi wiersza w pliku w postaci
zapisanej, bez zakończenia wiersza, także dla plików, które nie są w UTF-8.
Są pomijane w rekordach `content` zawierających zakres wierszy.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik do odczytu w postaci źródłowej. |
| `range` | `range` | `all` | Zakres wierszy. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| `source` | `none` | `line` |
| `stage` | `entry`, `content` | `line` |

Pola `line`: `path`, `no`, `text` (bez zakończenia wiersza),
`line_ending` (`lf`, `crlf`, `cr` lub `none`), `etag`, `start`, `end`,
`encoding`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie drugiego i trzeciego wiersza pliku.

```nurt example
lines "/home/user/app/notes.txt" 2..3
```

```text expected
/home/user/app/notes.txt:2: beta
/home/user/app/notes.txt:3: gamma
```

Podział wierszy odczytanych przez `read` z zachowaniem numerów wierszy pliku.

```nurt example
read "/home/user/app/notes.txt" lines: 3..4 |> lines
```

```text expected
/home/user/app/notes.txt:3: gamma
/home/user/app/notes.txt:4: delta
```

Wyświetlenie pierwszego wiersza każdego znalezionego pliku tekstowego.

```nurt example
find "/home/user/app" glob: "*.txt" |> lines 1
```

Odpowiedź, która pomija wiersze, kończy się wskazówką „more: next” i zawiera
pole `continuation`; polecenie `next` zwraca kolejną część bez ponownego
uruchamiania polecenia.

```text
shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Brak pliku w postaci źródłowej. | Podaj `path`. |
| `E_INVALID_VALUE` | Granica `range` jest mniejsza niż 1. | Numeruj wiersze od 1, jak w `1..20`. |
| `E_TYPE` | `range` nie jest zakresem albo rekord wejściowy nie jest ani `entry`, ani `content`. | Podaj zakres, taki jak `10..20`, albo przekaż rekordy plików. |
| `E_BINARY` | Plik postaci źródłowej ma zawartość binarną. | Dla plików binarnych użyj `read view: hex`. |
| `E_ENCODING` | Zawartość jest nieprawidłowa w wykrytym kodowaniu. | Odczytaj plik z `read encoding:` i przekonwertuj go poleceniem `convert`. |
| `E_LIMIT` | Plik jest większy niż limit tekstu. | Odczytaj zakres wierszy poleceniem `read lines:`. |
| `E_NOT_FOUND` | Plik nie istnieje. | Sprawdź ścieżkę. |
| `E_PATH_RELATIVE` | Podano ścieżkę względną bez katalogu bazowego. | Podaj ścieżkę bezwzględną albo ustaw opcję `base`. |

## Powiązane {#related}

`read`, `search`, `replace`, `insert`.
