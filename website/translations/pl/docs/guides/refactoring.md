---
name: refactoring
description: Zmiana nazw i modyfikowanie kodu według składni i symboli zamiast według tekstu.
commands: [code.symbols, code.references, code.rename, code.diagnostics, code.outline, search, replace, verify, run, assert]
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->
<!-- -->
<!-- Module:           docs/guides/refactoring.md -->
<!-- Responsibility:   Documents refactoring. -->
<!-- Dependencies:     none -->
<!-- Author:           Danaco Official -->
<!-- Version:          0.1.1, 2026-11-27 -->
<!-- License:          see the LICENSE file or https://www.danaco-group.pl -->

# refaktoryzacja {#refactoring}

## Tekst, składnia i symbole {#text-syntax-and-symbols}

`search` i `replace` działają na tekście. `search mode: syntax` dopasowuje wzorce składni, więc dopasowanie wewnątrz komentarza lub ciągu znaków nie jest mylone z kodem, a `scope: code` ogranicza wyszukiwanie tekstu do kodu. Polecenia `code.*` działają na symbolach: symbol ma definicję i odwołania, których wyszukiwanie tekstu nie odróżni od innych użyć tego samego słowa.

## Wyszukiwanie symboli {#finding-symbols}

`code.symbols` wyszukuje definicje według nazwy w całym repozytorium. `kind` zawęża wynik do funkcji, metod, klas i innych rodzajów symboli, a `match` wybiera porównanie dokładne, prefiksowe, podciągowe, rozmyte lub według wyrażenia regularnego. `code.outline` wyświetla symbole jednego pliku, a `code.references` wyświetla miejsca, które używają symbolu.

```nurt
code.symbols "oldName" path: ["/srv/app/src"] kind: function
code.references "/srv/app/src/a.go" line: 12 column: 6
```

## Zmiana nazwy {#renaming}

`code.rename` zmienia nazwę symbolu i jego odwołań we wszystkich plikach jako jedną operację, więc pojedyncze `undo` ją cofa. Symbol pochodzi z wejścia lub z argumentów `path`, `line` i `column`. Parametr `source` wybiera analizę: `lsp` używa zainstalowanego serwera języka, `syntax` używa lokalnej analizy składni, a `auto` wybiera pierwszą dostępną.

```nurt
code.symbols "oldName" path: ["/srv/app/src"] kind: function |> code.rename "newName"
```

## Sprawdzanie po zmianie {#checking-after-a-change}

Zmiana nazwy, która się kompiluje, nie dowodzi, że zachowanie pozostało niezmienione. `verify` uruchamia sprawdzenie po zmianie i wycofuje zmianę, gdy sprawdzenie zawiedzie. `code.diagnostics` zgłasza błędy i ostrzeżenia w zmienionych plikach.

```nurt
code.rename "newName" path: "/srv/app/src/a.go" line: 12 column: 6 |> verify (run ["go", "build", "./..."] cwd: "/srv/app" |> assert exit_code: 0)
```

## Ograniczenia {#limits}

Zamiana nazwy w tekście zmienia każde wystąpienie, także niezwiązane. Gdy zamiana w tekście jest jedyną możliwością, `check_references` zgłasza odwołania, które pozostałyby niezmienione, a `assert paths: N` zatrzymuje program, gdy liczba objętych plików różni się od oczekiwanej. Dynamiczne użycia nazwy, takie jak nazwy budowane z ciągów znaków, nie są odwołaniami i nie są zmieniane.

<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- Support: support@danaco-group.pl -->
