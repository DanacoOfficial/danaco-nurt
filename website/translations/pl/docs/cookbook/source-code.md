---
category: source-code
title: Kod źródłowy
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/cookbook/source-code.md -->
<!-- Responsibility:   Documents Source code. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.2, 2026-11-28 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->
# Kod źródłowy {#source-code}

## Wyszukiwanie definicji funkcji {#find-the-definition-of-a-function}

Znajdź symbole o nazwie `parseConfig`, które są funkcjami.

```nurt example
code.symbols "parseConfig" path: ["/home/user/app/src"] kind: function
```

```text expected
parseConfig
```

## Zmiana nazwy funkcji na poziomie składni {#rename-a-function-by-syntax}

Zmień nazwę funkcji i jej odwołań; ciągi znaków i komentarze nie są zmieniane.

```nurt example
code.symbols "oldName" path: ["/home/user/app/src"] kind: function |> code.rename "newName"
```

```text expected
newName
```

```bash
gopls rename -w /home/user/app/src/a.go:12:6 newName
```

## Wyświetlanie odwołań do symbolu {#list-the-references-of-a-symbol}

Wyświetl odwołania do symbolu w danej pozycji pliku.

```nurt example
code.references "/home/user/app/src/a.go" line: 12 column: 6
```

```text expected
/home/user/app/src/a.go
```

## Wyświetlanie zarysu pliku {#show-the-outline-of-a-file}

Pokaż symbole pliku źródłowego w postaci zarysu.

```nurt example
code.outline "/home/user/app/src/a.go"
```

```text expected
a.go
```

## Przejście do definicji w danej pozycji {#go-to-the-definition-at-a-position}

Znajdź definicję symbolu w danej pozycji.

```nurt example
code.definition "/home/user/app/src/a.go" line: 40 column: 9
```

```text expected
/home/user/app/src
```

## Wyświetlanie diagnostyki projektu {#show-the-diagnostics-of-a-project}

Pokaż błędy i ostrzeżenia plików źródłowych katalogu.

```nurt example
code.diagnostics ["/home/user/app/src"] severity: warning
```

```text expected
severity
```

## Wyszukiwanie implementacji interfejsu {#find-the-implementations-of-an-interface}

Znajdź typy implementujące metodę interfejsu.

```nurt example
code.implementations "/home/user/app/src/shape.go" line: 5 column: 6
```

```text expected
/home/user/app/src
```

## Wyświetlanie hierarchii wywołań funkcji {#show-the-call-hierarchy-of-a-function}

Pokaż funkcje wywołujące daną funkcję.

```nurt example
code.call_hierarchy "/home/user/app/src/a.go" line: 12 column: 6 direction: incoming
```

```text expected
/home/user/app/src
```

## Wyświetlanie języków z serwerem języka {#list-the-languages-with-a-language-server}

Wyświetl języki, dla których polecenia kodu mają analizator.

```nurt example
code.languages
```

```text expected
go
```

## Podsumowanie repozytorium {#summarize-a-repository}

Pokaż manifesty, punkty wejścia i języki repozytorium.

```nurt example
code.profile "/home/user/app" sections: [manifests, languages]
```

```text expected
manifests
```

## Przeszukiwanie kodu tylko w komentarzach {#search-code-inside-comments-only}

Wyszukaj wzorzec wyłącznie w komentarzach plików źródłowych.

```nurt example
search /deprecated/ "/home/user/app/src" scope: comments
```

```text expected
/home/user/app/src
```

## Zastąpienie tylko w kodzie {#replace-only-in-code}

Zastąp identyfikator w kodzie i pozostaw ciągi znaków oraz komentarze bez zmian.

```nurt example
replace "oldName" "newName" in: ["/home/user/app/src"] glob: "**/*.go" scope: code whole_word: true
```

```text expected
/home/user/app/src
```

## Uruchomienie testów projektu Go {#run-the-tests-of-a-go-project}

Uruchom testy projektu i wymagaj kodu zakończenia równego zero.

```nurt example
run ["go", "test", "./..."] cwd: "/home/user/app" |> assert exit_code: 0
```

```text expected
exit_code
```

## Sprawdzenie typów i stylu projektu {#check-types-and-style-of-a-project}

Uruchom kontrole analizatorów projektu wyłącznie dla zmienionych plików.

```nurt example
analyze.check changed_only: true
```

```text expected
finding
```

## Wyświetlanie eksportowanych funkcji wraz z sygnaturami {#list-the-exported-functions-with-their-signatures}

Wyświetl eksportowane funkcje, których nazwa zaczyna się od `New`.

```nurt example
code.symbols "New" match: prefix kind: function exported_only: true with_signature: true path: ["/home/user/app/src"]
```

```text expected
signature
```

## Wyświetlanie struktur poza testami {#list-the-structures-outside-the-tests}

Wyświetl struktury projektu i pomiń pliki testów.

```nurt example
code.symbols kind: struct path: ["/home/user/app/src"] exclude_preset: [tests]
```

```text expected
struct
```

## Wyświetlanie odwołań w obrębie jednego pliku {#list-the-references-within-one-file}

Wyświetl odwołania do symbolu wyłącznie w pliku jego definicji.

```nurt example
code.references "/home/user/app/src/a.go" line: 12 column: 6 scope: file include_definition: true
```

```text expected
/home/user/app/src/a.go
```

## Wyświetlanie akcji dostępnych dla zakresu wierszy {#list-the-actions-available-for-a-range-of-lines}

Wyświetl poprawki i refaktoryzacje oferowane przez analizator dla wierszy od 20 do 25.

```nurt example
code.actions "/home/user/app/src/a.go" line: 20 end_line: 25
```

```text expected
code_action
```

## Wyświetlanie błędów zmienionych plików {#show-the-errors-of-the-changed-files}

Pokaż wyłącznie błędy plików zmienionych od ostatniej rewizji.

```nurt example
code.diagnostics changed: true severity: error
```

```text expected
severity
```

## Wyświetlanie punktów wejścia i poleceń projektu {#show-the-entry-points-and-the-commands-of-a-project}

Pokaż, jak projekt jest budowany, testowany i uruchamiany.

```nurt example
code.profile "/home/user/app" sections: [commands, entry_points]
```

```text expected
commands
```

## Zmiana nazwy symbolu i jego wzmianek w komentarzach {#rename-a-symbol-and-its-mentions-in-comments}

Zmień nazwę funkcji i jej nazwę w komentarzach kodu.

```nurt example
code.symbols "oldName" path: ["/home/user/app/src"] kind: function |> code.rename "newName" include_comments: true
```

```text expected
newName
```

## Przeszukiwanie ciągów znaków plików źródłowych {#search-the-strings-of-the-source-files}

Znajdź adresy internetowe zapisane w literałach tekstowych kodu.

```nurt example
search /https?:\/\// "/home/user/app/src" scope: strings
```

```text expected
/home/user/app/src
```

## Sprawdzenie typów i linter dla całego projektu {#check-types-and-lint-for-the-whole-project}

Uruchom sprawdzenie typów i linter i zakończ błędem wyłącznie przy błędach.

```nurt example
analyze.check ["/home/user/app/src"] checks: [types, lint] fail_on: error
```

```text expected
analysis
```

## Odczyt ustaleń analizy {#read-the-findings-of-an-analysis}

Odczytaj co najwyżej 20 błędów wcześniejszej analizy.

```nurt example
analyze.show "a1" min_severity: error max_findings: 20
```

```text expected
finding
```

## Powtórzenie nieudanych testów {#repeat-the-failed-tests}

Uruchom ponownie tylko testy, które zakończyły się niepowodzeniem we wcześniejszym uruchomieniu.

```nurt example
test.failed "r1" failures: 10 message_lines: 5
```

```text expected
test_run
```

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
