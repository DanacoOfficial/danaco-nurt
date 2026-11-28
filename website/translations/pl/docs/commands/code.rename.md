---
command: code.rename
group: code
summary: Zmienia nazwę symbolu i jego odwołań w wielu plikach w jednej operacji.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.rename

## Składnia {#synopsis}

```text
symbol |> code.rename <to> [name: value]...
code.rename <to> path: <path> [name: value]...
```

## Opis {#description}

Zmienia nazwę jednego symbolu i każdego odwołania do niego. Postać źródła
wskazuje symbol za pomocą `path` z `line` i `column` albo za pomocą `symbol`,
czyli nazwy lub nazwy kwalifikowanej, takiej jak `Parser.parse`. Bez `column`
używany jest pierwszy identyfikator wiersza `line`. Postać etapu zmienia nazwę
każdego wejściowego rekordu `symbol`.

Edycje pochodzą z serwera języka (`source: lsp`) albo z analizy składniowej
(`source: syntax`), która zastępuje cały identyfikator w kodzie plików tej samej
rodziny języków (TypeScript i JavaScript tworzą jedną rodzinę, C i C++ drugą).
`source: auto` używa serwera, gdy jest dostępny, a w przeciwnym razie analizy
składniowej; serwer, który zawodzi, przekracza czas albo nie zwraca edycji, jest
zgłaszany jako `semantic_fallback`. Wystąpienia w komentarzach i literałach
tekstowych są zmieniane wyłącznie z `include_comments` i `include_strings`.

Plan jest sprawdzany przed zapisaniem czegokolwiek:

- edycja poza zakresem nie jest stosowana i jest zgłaszana jako
  `outside_project` (poza `root` i `paths`) lub `rename_incomplete` (inny plik
  przy `scope: file`);
- zmieniany plik, który już używa nowej nazwy w kodzie, kończy się błędem
  `E_RENAME_CONFLICT`;
- analiza składniowa, która znajdzie więcej niż jedną definicję nazwy, kończy
  się błędem `E_SYMBOL_AMBIGUOUS`, ponieważ zmieniona zostałaby każda
  definicja;
- edycje serwera, które robią więcej niż zastąpienie starej nazwy, są
  wymieniane w polu `alarms` podsumowania planu.

Wynik zawiera jeden rekord `rename_edit` na edycję oraz rekord `map`
z podsumowaniem planu: `plan` (identyfikator), `from`, `to`, `source`, `server`,
`confidence` (`semantic` dla edycji serwera, `identifier` dla analizy
składniowej), `root`, `files`, `total`, `alarms` i `leftovers`.

Wykonanie zapisuje każdy plik w jednej operacji dziennika, razem z tworzeniem,
zmianą nazwy i usuwaniem plików żądanymi przez serwer. Plik zmieniony po
odczytaniu kończy się błędem `E_STALE` przed zapisaniem czegokolwiek. Po
zmianie błędy składni i błędy serwera języka w każdym zapisanym pliku są
porównywane z błędami sprzed zmiany; nowy błąd przywraca każdy plik i kończy się
błędem `E_ROLLED_BACK`. Przy `check_references` wystąpienia starej nazwy
pozostałe w kodzie, komentarzach, ciągach znaków, dokumentacji, konfiguracji
i nazwach plików są zgłaszane jako `references_left`; wystąpienia pozostałe
w kodzie tej samej rodziny języków są zgłaszane także jako `rename_incomplete`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `to` | `text` | wymagany | Nowa nazwa symbolu. |
| `path` | `path` | brak | Plik źródłowy symbolu w postaci źródła. |
| `line` | `int` | brak | Wiersz symbolu w postaci źródła, liczony od 1. |
| `column` | `int` | brak | Kolumna symbolu w postaci źródła, liczona od 1. |
| `symbol` | `text` | brak | Nazwa symbolu, opcjonalnie kwalifikowana jako Container.name, zamiast położenia. |
| `scope` | `enum(project\|file)` | `project` | Zmienia nazwę w całym projekcie albo tylko w pliku. |
| `paths` | `list<path>` | projekt | Pliki i katalogi, w których zmieniana jest nazwa. |
| `include_comments` | `bool` | `false` | Zmienia także wystąpienia w komentarzach. |
| `include_strings` | `bool` | `false` | Zmienia także wystąpienia w literałach tekstowych. |
| `source` | `enum(auto\|lsp\|syntax)` | `auto` | Serwer języka, analiza składniowa z poziomem pewności albo serwer, gdy jest dostępny. |
| `check_references` | `bool` | `true` | Zgłasza odwołania do starej nazwy pozostałe po zmianie. |
| `language` | `text` | `"auto"` | Język programowania; auto wykrywa go na podstawie rozszerzenia pliku. |
| `root` | `path` | wykrywany na podstawie położenia pliku | Katalog główny projektu dla serwera języka. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| `stage` | `symbol` | `effect` |
| `source` | `none` | `effect` |

Rekord `effect` ma `kind: "code.rename"`, zmienione pliki w `changes`,
liczniki `files_modified`, `files_created`, `files_deleted`, `files_renamed`
i `replacements` w `summary` oraz kody skutków szczególnych w `notable`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `language_server` |
| Warunki odwracalności | Wszystkie zmienione pliki są przywracane razem jednym cofnięciem. |
| Rozwiązanie zapasowe | `builtin` |

## Przykłady {#examples}

Zmiana nazwy funkcji `total` zdefiniowanej w `math.py`.

```nurt example
code.rename "sum_all" path: "/srv/app/src/math.py" symbol: "total"
```

Zmiana nazwy symbolu w wierszu 12, kolumnie 5, także w komentarzach.

```nurt example
code.rename "parse_header" path: "/srv/app/src/reader.py" line: 12 column: 5 include_comments: true
```

Zmiana nazwy metody znalezionej po nazwie, z analizą składniową ograniczoną do
jednego katalogu.

```nurt example
code.symbols "loadConfig" path: ["/srv/app/src"] kind: method match: exact |> code.rename "readConfig" source: syntax paths: ["/srv/app/src"]
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | `to` nie jest identyfikatorem albo jest równe bieżącej nazwie. | Podaj inną nazwę. |
| `E_MISSING_PARAM` | Nie podano ani `symbol`, ani `line`. | Podaj `symbol` albo `line` i `column`. |
| `E_SYMBOL_NOT_FOUND` | Brak identyfikatora w tym położeniu, brak definicji `symbol` albo serwer nie znalazł niczego do zmiany. | Sprawdź położenie lub nazwę. |
| `E_SYMBOL_AMBIGUOUS` | `symbol` ma w pliku kilka definicji albo analiza składniowa znalazła kilka definicji nazwy; `candidates` wymienia jedno wywołanie dla każdej definicji. | Podaj `line` i `column`, zawęź `paths` albo użyj `source: lsp`. |
| `E_RENAME_CONFLICT` | Nowa nazwa jest już używana w zmienianym pliku. | Wybierz inną nazwę. |
| `E_UNSUPPORTED_LANGUAGE` | Brak gramatyki dla pliku i brak serwera. | Podaj `language` albo zainstaluj serwer. |
| `E_EXTERNAL_MISSING` | Przy `source: lsp` brak serwera dla języka. | Zainstaluj serwer albo użyj `source: syntax`. |
| `E_EXTERNAL_FAILED` | Przy `source: lsp` serwer zakończył się awarią. | Zobacz komunikat serwera. |
| `E_STALE` | Plik zmienił się po sporządzeniu planu. | Uruchom zmianę nazwy ponownie. |
| `E_ROLLED_BACK` | Zmiana nazwy wprowadziła nowy błąd; każdy plik został przywrócony. | Przejrzyj zgłoszony błąd. |
| `E_LIMIT` | Zmieniłoby się więcej niż 500 plików albo plik przekracza `code.file_bytes_max`. | Zawęź `paths` albo zwiększ limit. |

## Powiązane {#related}

`code.references`, `code.symbols`, `code.diagnostics`, `undo`,
`help skill: "refactoring"`.
