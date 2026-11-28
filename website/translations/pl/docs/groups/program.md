---
group: program
summary: "Programy zainstalowane w systemie: wyszukiwanie lokalizacji, przeszukiwanie, nawigacja i opisy."
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# Grupa program

## Zakres {#scope}

Grupa odpowiada na pytanie, jakie programy istnieją na komputerze i jak ich
używać. Lokalizuje programy w ścieżce wyszukiwania, przeszukuje zainstalowane
programy i katalogi programów, opisuje program wraz z jego wersją i
przewodnikiem użycia oraz przegląda programy według kategorii.

Grupa działa na każdym komputerze od pierwszego wywołania i nie wymaga
konfiguracji. Katalogi zewnętrzne są opcjonalne i dodają opisy, kategorie i
przewodniki użycia do programów, które system już zgłasza.

| Zadanie | Polecenie | Grupa |
|---|---|---|
| Uruchomienie programu | `run` | core |
| Wylistowanie, instalacja i usunięcie pakietów | `package.*` | package |
| Opis hosta i jego systemu operacyjnego | `host` | system |
| Odczyt strony podręcznika programu jako pliku | `read` | core |

`package.search` odpytuje repozytoria menedżera pakietów i wymaga menedżera.
Polecenia tej grupy odczytują bazy menedżerów pakietów i nigdy nie uruchamiają
menedżera.

## Polecenia {#commands}

| Polecenie | Kategoria | Wejście i wyjście | Odwracalność | Strona |
|---|---|---|---|---|
| `program.find` | source | brak do `program_location` | read | [program.find](../commands/program.find.md) |
| `program.search` | source | brak do `program_card` | read | [program.search](../commands/program.search.md) |
| `program.describe` | source | brak do `program_card` | read | [program.describe](../commands/program.describe.md) |
| `program.catalog` | source | brak do `program_category` lub `program_card` | read | [program.catalog](../commands/program.catalog.md) |

## Rekordy {#records}

| Rekord | Wymagane pola |
|---|---|
| `program_location` | `name`, `found` |
| `program_card` | `name`, `summary`, `source`, `installed`, `guide_optional`, `sources` |
| `program_category` | `name`, `count`, `source` |

`program_location` zgłasza ścieżkę, na którą rozwiązuje się nazwa; `all_paths`
wylicza każde dopasowanie w kolejności, w jakiej system je rozstrzyga.
`program_card` zawiera opis w `description` i przewodnik użycia w `guide`.
Pole `source` wskazuje główne źródło (`path`, `package`, `app` lub `catalog`),
a `sources` wylicza każde źródło, z którego złożono kartę, dzięki czemu znane
jest pochodzenie każdego tekstu.

## Źródła danych {#sources-of-data}

| Źródło | Wartość `source` | Zawartość |
|---|---|---|
| Ścieżka wyszukiwania | `path` | nazwa, ścieżka i wersja plików wykonywalnych; w systemie Windows uwzględniane jest `PATHEXT` |
| Bazy pakietów | `package` | nazwa, wersja, opis i pliki wykonywalne zainstalowanych pakietów |
| Aplikacje systemowe | `app` | nazwa, ścieżka, wersja i wydawca aplikacji |
| Katalogi programów | `catalog` | opisy, kategorie i przewodniki użycia |
| Opisy | | strony podręcznika i wpisy `whatis`, metadane pakietów |

Bazy pakietów są odczytywane z plików, bez uruchamiania menedżera:

| System | Bazy |
|---|---|
| Linux | dpkg, rpm, pacman |
| macOS | Homebrew |
| Windows | winget, Scoop, Chocolatey |

Aplikacje pochodzą z rejestru i menu Start w systemie Windows, z pakietów
aplikacji w systemie macOS oraz z wpisów desktop w systemie Linux.

## Katalogi zewnętrzne {#external-catalogs}

Katalog jest katalogiem plików w formacie `nurt-program-catalog/1`:

```text
catalog.json        index: format, categories and programs
guides/<name>.md    usage guide of a program, Markdown with a front matter
```

Indeks wylicza kategorie (`id`, `title`, `description`, `parent`) i programy
(`name`, `category`, `summary`, `aliases`, `platforms`, `guide`, `verified`,
`tags`). Czytnik traktuje katalog jako dane: nie wykonuje żadnego pola ani
pliku katalogu i nie pobiera żadnego adresu.

Ustawienie `programs.catalogs` wylicza katalogi: ścieżki lub tabele z
`path`, `priority` i `enabled`. Wartością domyślną jest pusta lista, a produkt
nie wskazuje żadnego katalogu. Bez katalogów polecenia opisują zainstalowane
programy. Katalog skonfigurowany jawnie, którego nie można odczytać, kończy się
błędem `E_CATALOG`. Ustawienie `programs.sources_disabled` wyklucza źródła
(`path`, `package`, `app`, `man`, `catalog`) z indeksu.

## Różnice między platformami {#platform-differences}

| Aspekt | Linux | macOS | Windows |
|---|---|---|---|
| Pliki wykonywalne | pliki z uprawnieniem wykonywania | pliki z uprawnieniem wykonywania | rozszerzenia z `PATHEXT` |
| Źródło aplikacji | wpisy desktop | pakiety aplikacji | rejestr i menu Start |
| Bazy pakietów | dpkg, rpm, pacman | Homebrew | winget, Scoop, Chocolatey |
| Wersja pliku wykonywalnego | wersja pakietu | wersja pakietu aplikacji | zasób wersji pliku |

`E_OS_ONLY` nie jest zgłaszany przez tę grupę: każde polecenie działa we
wszystkich trzech systemach, a źródło, które nie istnieje w systemie, jest
pomijane.

## Programy zewnętrzne {#external-programs}

| Polecenie | Znacznik | Warunek |
|---|---|---|
| `program.describe` | [U] | uruchamia program jednokrotnie z `--help`, w granicach limitu czasu, gdy używane jest `help` |

Limit czasu odczytu pomocy to ustawienie `programs.help_timeout` (wartość
domyślna `3s`). Żadne inne polecenie grupy nie uruchamia programu, a indeks
nigdy nie jest budowany przez uruchamianie programów. Znacznik [U] wskazuje
program użytkownika.

## Przykłady {#examples}

Zlokalizowanie dwóch programów i zgłoszenie ich wersji:

```nurt example
program.find ["git", "python3"] versions: true
```

Przeszukanie zainstalowanych programów i katalogów oraz zachowanie
zainstalowanych:

```nurt example
program.search "convert images" limit: 10 |> filter installed |> select name path summary
```

Przeszukanie tylko ścieżki wyszukiwania i baz pakietów:

```nurt example
program.search "pdf" source: [path, package] limit: 5
```

Opis programu bez jego przewodnika użycia:

```nurt example
program.describe "ffmpeg" include_guide: false
```

Wylistowanie kategorii z największą liczbą programów:

```nurt example
program.catalog |> sort count order: desc |> first 5
```

## Powiązane {#related}

[Grupa package](package.md) instaluje programy, które ta grupa znajduje.
[Grupa system](system.md) zgłasza hosta, na którym działa wyszukiwanie.
