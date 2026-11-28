---
command: open_files
group: system
summary: Wyświetla procesy, które mają otwarty plik lub katalog.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# open_files

## Składnia {#synopsis}

```text
open_files <path>
```

## Opis {#description}

`open_files` zwraca jeden rekord `process` dla każdego procesu, który trzyma
`path`, uporządkowany według `pid`. Ścieżka musi być bezwzględna i musi
istnieć; dowiązania symboliczne w niej są najpierw rozwiązywane. Plik pasuje do
procesów, które używają tego pliku. Katalog pasuje do procesów, które używają
tego katalogu albo dowolnej ścieżki poniżej niego.

| System | Źródło | `os_raw.access` |
|---|---|---|
| Linux | otwarte deskryptory, katalogi robocze i główne, plik wykonywalny i mapy pamięci w `/proc` | `fd`, `cwd`, `root`, `exe`, `mmap` |
| macOS | otwarte deskryptory, katalogi robocze i główne, plik wykonywalny i zmapowane obszary zgłaszane przez jądro | `fd`, `cwd`, `root`, `exe`, `mmap` |
| Windows | Restart Manager | nieustawione; `os_raw.kind` to `application`, `service`, `explorer`, `console`, `critical` lub `unknown` |

`os_raw.files` wymienia pasujące ścieżki używane przez proces. Plik usunięty,
gdy proces nadal go trzyma otwartego, pasuje do katalogu, który go zawierał;
rekord ma wtedy `os_raw.deleted: true`. Plik znajdujący się teraz pod tą samą
ścieżką jest innym plikiem i nie pasuje.

Procesy, których otwartych plików nie można odczytać, zazwyczaj procesy innych
kont, są pomijane; raport zawiera jedną uwagę `permission_denied_skipped` z ich
liczbą. Uruchom konsolę jako root lub jako administrator, aby je uwzględnić. W
systemie Windows katalog jest sprawdzany przez zwykłe pliki poniżej niego, do
10 000; powyżej tej liczby raport zawiera uwagę `target_count_high`.

Warstwa plików używa tego samego wyszukiwania do wskazania posiadaczy pliku w
błędzie `E_IN_USE`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `path` | wymagany | Plik lub katalog. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `process` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie procesów, które trzymają plik dziennika:

```nurt example
open_files "/var/log/app/current.log"
```

```text expected
name: app
user: app
```

Znalezienie procesów, które zajmują zamontowany wolumin:

```nurt example
open_files "/mnt/backup" |> select pid name user
```

Znalezienie programów, które blokują plik w systemie Windows:

```nurt example
open_files "C:/Users/user/Documents/report.xlsx"
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_PATH_RELATIVE` | `path` jest względna. | Podaj ścieżkę bezwzględną. |
| `E_NOT_FOUND` | `path` nie istnieje. | Sprawdź ścieżkę za pomocą `info`. |
| `E_PRIVILEGE` | Tablica procesów lub Restart Manager odmawia dostępu. | Uruchom konsolę jako root lub jako administrator. |
| `E_IO` | Nie można odczytać tablicy procesów. | Sprawdź, czy `/proc` jest zamontowany. |

## Powiązane {#related}

`processes`, `ports`, `stop`, `accounts`.
