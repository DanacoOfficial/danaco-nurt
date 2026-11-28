---
command: code.diagnostics
group: code
summary: Zgłasza błędy i ostrzeżenia w plikach źródłowych.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.diagnostics

## Składnia {#synopsis}

```text
code.diagnostics <path>... [name: value]...
```

## Opis {#description}

Zgłasza błędy i ostrzeżenia podanych plików. Katalog oznacza pliki źródłowe
i dokumenty strukturalne poniżej niego, z pominięciem `.git`, `node_modules`,
`__pycache__`, `venv` i `.venv`, do `code.index_max_files` plików. Przy
`changed: true` sprawdzane są tylko pliki zmienione w repozytorium: zmiany
w drzewie roboczym i indeksie oraz pliki nieśledzone, ograniczone do podanych
ścieżek, jeżeli jakiekolwiek podano.

Każdy plik jest sprawdzany przez swój serwer języka. Serwer obsługujący
diagnostyki pobierane na żądanie jest o nie pytany; dla pozostałych klient
otwiera plik i czeka, aż serwer przez `code.diagnostics_quiet` (1500 ms) nie
opublikuje niczego. Przy `source: auto` plik bez użytecznego serwera jest
sprawdzany analizą składniową: gramatyką jego języka albo, dla dokumentów
strukturalnych (JSON, YAML, TOML i pozostałe formaty dokumentów), parserem
formatu. Każda odrębna przyczyna użycia rozwiązania zapasowego jest zgłaszana
jeden raz jako notatka `semantic_fallback`.

Rekordy nie zawierają powtórzeń i są uporządkowane według poziomu ważności,
ścieżki, wiersza i kolumny. Dla więcej niż pięciu plików polecenie zgłasza swój
postęp.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `list<path>` | brak | Pliki lub katalogi do sprawdzenia. |
| `changed` | `bool` | `false` | Tylko pliki zmienione w repozytorium. |
| `severity` | `enum(error\|warning\|info\|hint)` | `hint` | Najniższy zgłaszany poziom ważności. |
| `source` | `enum(auto\|lsp\|syntax)` | `auto` | Serwer języka, analiza składniowa albo serwer, gdy jest dostępny. |
| `language` | `text` | `"auto"` | Język programowania; auto wykrywa go na podstawie rozszerzeń plików. |
| `root` | `path` | wykrywany na podstawie położenia pliku | Katalog główny projektu dla serwera języka. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `diagnostic` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `language_server` |
| Rozwiązanie zapasowe | `builtin` |

## Przykłady {#examples}

Sprawdzenie jednego pliku.

```nurt example
code.diagnostics "/srv/app/src/main.py"
```

Zgłoszenie wyłącznie błędów katalogu.

```nurt example
code.diagnostics "/srv/app/src" severity: error
```

Sprawdzenie plików zmienionych w repozytorium i wyświetlenie plików z błędami.

```nurt example
code.diagnostics changed: true |> filter severity == "error" |> select path line message
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Nie podano ścieżki, a `changed` ma wartość `false`. | Podaj ścieżkę albo `changed: true`. |
| `E_INVALID_VALUE` | `severity` lub `source` jest niepoprawne albo ścieżka początkowa dla `changed: true` nie leży w repozytorium. | Sprawdź parametry. |
| `E_ARG` | `changed: true` bez ścieżki i bez katalogu bazowego. | Podaj ścieżkę wewnątrz repozytorium. |
| `E_LIMIT` | Plik jest większy niż `code.file_bytes_max`. | Zwiększ limit albo pomiń plik. |
| `E_ENCODING` | Plik nie jest tekstem UTF-8. | Przekonwertuj plik na UTF-8. |
| `E_EXTERNAL_MISSING` | Przy `source: lsp` nie ma dostępnego serwera albo przy `changed: true` zapytania o repozytorium są niedostępne. | Zainstaluj serwer wskazany w `fix` albo podaj ścieżki. |
| `E_EXTERNAL_FAILED` | Przy `source: lsp` serwer zakończył się awarią. | Zobacz komunikat serwera. |
| `E_TIMEOUT` | Przy `source: lsp` serwer nie odpowiedział w czasie `code.timeout`. | Zwiększ `code.timeout` albo użyj `source: auto`. |

## Powiązane {#related}

`code.languages`, `validate`, klucze konfiguracji `code.diagnostics_quiet`,
`code.timeout`.
