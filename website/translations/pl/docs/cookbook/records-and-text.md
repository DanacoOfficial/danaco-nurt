---
category: records-and-text
title: Rekordy i przetwarzanie tekstu
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cookbook/records-and-text.md -->
<!-- Responsibility:   Documents Records and text processing. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Rekordy i przetwarzanie tekstu {#records-and-text-processing}

## Sortowanie rekordów według pola {#sort-records-by-a-field}

Posortuj wpisy katalogu według rozmiaru, od największego.

```nurt example
list "/home/user/app" |> sort size order: desc
```

```text expected
/home/user/app
```

## Pobranie pierwszych rekordów {#take-the-first-records}

Pobierz dziesięć ostatnio zmodyfikowanych plików.

```nurt example
find "/home/user/app" |> sort modified order: desc |> first 10
```

```text expected
/home/user/app
```

## Wybór pól {#select-fields}

Pokaż tylko ścieżkę i rozmiar plików.

```nurt example
find "/home/user/app" |> select path size
```

```text expected
path
```

## Obliczenie pola {#compute-a-field}

Pokaż rozmiar plików w kilobajtach.

```nurt example
find "/home/user/app" |> select path kb: size / 1000
```

```text expected
kb
```

## Grupowanie i podsumowanie {#group-and-summarize}

Policz pliki według rozszerzenia.

```nurt example
find "/home/user/app" |> group by: extension |> summarize
```

```text expected
extension
```

## Usunięcie duplikatów {#remove-duplicates}

Wyświetl różne rozszerzenia plików projektu.

```nurt example
find "/home/user/app" |> select extension |> distinct
```

```text expected
extension
```

## Pominięcie nagłówka {#skip-a-header}

Pomiń pierwszy wiersz pliku.

```nurt example
lines "/home/user/data.csv" |> skip 1 |> first 5
```

```text expected
data.csv
```

## Podział wierszy na pola {#split-lines-into-fields}

Podziel wiersze pliku rozdzielanego dwukropkami na nazwane pola.

```nurt example
lines "/etc/example/passwd" |> split ":" names: ["user", "uid", "home"]
```

```text expected
user
```

```bash
awk -F: '{print $1, $3}' /etc/example/passwd
```

## Analiza wierszy wzorcem {#parse-lines-with-a-pattern}

Wyodrębnij nazwane części każdego wiersza logu.

```nurt example
lines "/home/user/app/app.log" |> parse /(?P<level>\w+) (?P<message>.*)/
```

```text expected
level
```

## Opis pól rekordów {#describe-the-fields-of-records}

Pokaż pola i typy rekordów polecenia.

```nurt example
processes |> describe
```

```text expected
field
```

## Obliczenie wyrażenia {#evaluate-an-expression}

Oblicz wyrażenie bez odczytu jakiegokolwiek pliku.

```nurt example
evaluate 40 + 2
```

```text expected
42
```

## Nazwanie wyniku i dwukrotne użycie {#name-a-result-and-use-it-twice}

Zapisz wynik wyszukiwania i policz go.

```nurt example
let recent = find "/home/user/app/src" glob: "**/*.ts" |> filter modified > ago(1h)
@recent |> count
```

```text expected
0
```

## Obliczenie skrótu pliku {#hash-a-file}

Oblicz skrót SHA-256 pliku.

```nurt example
hash "/home/user/dl/a.tar.gz" algorithm: sha256
```

```text expected
sha256
```

```bash
sha256sum /home/user/dl/a.tar.gz
```

## Filtrowanie według wzorca w polu {#filter-by-a-pattern-in-a-field}

Zachowaj wpisy, których nazwa pasuje do wyrażenia regularnego.

```nurt example
list "/home/user/app" |> filter name ~ /^test_/
```

```text expected
test_
```

## Sortowanie według dwóch pól {#sort-by-two-fields}

Posortuj pliki według rozszerzenia, a w obrębie rozszerzenia według rozmiaru.

```nurt example
find "/home/user/app" |> sort by: [extension, size] order: desc
```

```text expected
extension
```

## Pobranie ostatnich rekordów {#take-the-last-records}

Pobierz pięć najstarszych plików katalogu.

```nurt example
find "/home/user/app" |> sort modified |> first 5
```

```text expected
/home/user/app
```

## Zachowanie ostatnich wierszy pliku {#keep-the-last-lines-of-a-file}

Zachowaj trzy ostatnie wiersze pliku.

```nurt example
lines "/home/user/app/app.log" |> last 3
```

```text expected
app.log
```

## Filtrowanie wierszy pliku {#filter-the-lines-of-a-file}

Zachowaj wiersze pliku zawierające ostrzeżenie.

```nurt example
lines "/home/user/app/app.log" |> filter text ~ /WARN/
```

```text expected
WARN
```

## Wybór numeru i tekstu wierszy {#select-the-number-and-the-text-of-lines}

Pokaż numer i tekst każdego wiersza.

```nurt example
lines "/home/user/app/main.go" |> select no text
```

```text expected
text
```

## Zliczanie rekordów według pola {#count-the-records-by-a-field}

Policz pliki każdego typu.

```nurt example
find "/home/user/app" |> group by: type |> summarize
```

```text expected
type
```

## Sumowanie pola {#sum-a-field}

Zsumuj rozmiary plików według rozszerzenia.

```nurt example
find "/home/user/app" |> summarize size by: extension
```

```text expected
size
```

## Podział tabeli na pola {#split-a-table-into-fields}

Podziel wiersze pliku rozdzielanego średnikami i zachowaj dwie kolumny.

```nurt example
lines "/home/user/data.csv" |> skip 1 |> split ";" names: ["id", "name", "city"] |> select cells.id cells.name
```

```text expected
id
```

## Zapytanie do bazy SQLite {#query-a-sqlite-database}

Odczytaj wiersze tabeli z pliku bazy danych.

```nurt example
sql "/home/user/app/data.db" "select id, name from users where active = 1"
```

```text expected
name
```

## Zapytanie z nazwanymi parametrami {#query-with-named-parameters}

Powiąż wartości zapytania według nazwy.

```nurt example
sql "/home/user/app/data.db" "select id from orders where total > :min and city = :city" params: {min: 100, city: "Gdansk"}
```

```text expected
id
```

## Zmiana wierszy i przegląd wyniku {#change-rows-and-review-the-result}

Zaktualizuj wiersze pliku bazy danych i pokaż liczbę zmienionych wierszy.

```nurt example
sql "/home/user/app/data.db" "update users set active = 0 where last_seen < ?" params: ["2025-01-01"]
```

```text expected
rows_changed
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
