---
command: code.profile
group: code
summary: Tworzy profil repozytorium: polecenia kompilacji i testów, punkty wejścia oraz miejsca największych zmian na podstawie historii i manifestów.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# code.profile

## Składnia {#synopsis}

```text
code.profile <root> [name: value]...
```

## Opis {#description}

Opisuje projekt w katalogu `root`, niczego w nim nie uruchamiając ani nie
zmieniając. Wynikiem jest jeden rekord `profile_entry` na wpis, pogrupowany
według sekcji w kolejności `languages`, `modules`, `entry_points`, `commands`,
`instructions`, `hotspots`, `manifests`, poprzedzony rekordem nagłówka.

| Sekcja | Wpisy |
|---|---|
| `languages` | Języki plików, według liczby plików. |
| `modules` | Katalogi zawierające manifest projektu. |
| `entry_points` | Programy i biblioteki, od których projekt się zaczyna: `main`, `module`, `bin` i `exports` z `package.json`, `[[bin]]` i `src/main.rs` z `Cargo.toml`, `[project.scripts]` z `pyproject.toml`, `cmd/<name>/main.go` oraz pliki o konwencjonalnych nazwach, takie jak `__main__.py`, `main.py`, `main.go` i `index.js`. |
| `commands` | Polecenia kompilacji, testów i kontroli ze skryptów `package.json`, `Cargo.toml`, `Makefile`, `justfile`, `pyproject.toml`, `setup.cfg`, `tox.ini`, `go.mod`, `CMakeLists.txt`, `build.gradle`, `build.gradle.kts`, `pom.xml`, `*.csproj`, `composer.json`, `Gemfile` i `deno.json`. |
| `instructions` | Pliki z instrukcjami dla współtwórców i narzędzi (`AGENTS.md`, `CLAUDE.md` i podobne). |
| `hotspots` | Pliki z największą liczbą commitów w ciągu ostatnich `days` dni, a następnie pliki z niezatwierdzonymi zmianami. Poza repozytorium najpóźniej zmodyfikowane pliki. Pliki blokad zależności i wynik kompilacji są pomijane. |
| `manifests` | Każdy znaleziony manifest. |

Każde polecenie i każdy punkt wejścia wskazuje swoje pochodzenie: manifest,
a po nim klucz lub wiersz, z którego został odczytany (`package.json:scripts.test`,
`Makefile:12`). Ścieżki są bezwzględne.

Manifesty JSON i TOML są odczytywane przez usługę dokumentów; bez niej
manifesty JSON są odczytywane bezpośrednio, a manifesty TOML dostarczają
wyłącznie polecenia wynikające z ich obecności, z notatką w raporcie. Historia
i stan pochodzą z usługi zapytań o repozytorium.

Profil jest buforowany w katalogu stanu. Zbuforowany profil jest używany, dopóki
jest młodszy niż dziesięć minut, repozytorium jest na tym samym commicie, okres
historii jest taki sam i żaden manifest, z którego go odczytano, nie uległ
zmianie. Niezatwierdzone zmiany są odczytywane przy każdym wywołaniu.
`refresh: true` odczytuje drzewo ponownie.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `root` | `path` | wymagany | Katalog główny repozytorium. |
| `sections` | `list<enum(commands\|entry_points\|hotspots\|manifests\|languages\|modules\|instructions)>` | brak | Sekcje do zwrócenia; po pominięciu wszystkie. |
| `limit` | `int` | brak | Maksymalna liczba wpisów na sekcję. |
| `days` | `int` | `90` | Liczba dni historii używana do wyznaczania miejsc największych zmian. |
| `refresh` | `bool` | `false` | Odbudowuje profil zamiast używać zbuforowanego. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `profile_entry` |

Rekord nagłówka ma pola `root`, `revision` (commit albo null poza repozytorium),
`repository`, `files` (liczba wymienionych plików), `truncated` (wyliczanie
zatrzymało się na 50 000 plików), `cached` oraz, dla profilu zbuforowanego,
`cache_age_ms`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Profil projektu w `/srv/app`.

```nurt example
code.profile "/srv/app"
```

```text expected
section       name    command         origin
commands      build   npm run build   package.json:scripts.build
commands      test    npm run test    package.json:scripts.test
```

Wyświetlenie wyłącznie poleceń kompilacji i testów.

```nurt example
code.profile "/srv/app" sections: [commands]
```

Pokazanie pięciu plików zmienianych najczęściej w ciągu ostatnich 30 dni.

```nurt example
code.profile "/srv/app" sections: [hotspots] days: 30 limit: 5 |> select name changes last_change
```

Ponowne odczytanie profilu po zmianie ustawień kompilacji.

```nurt example
code.profile "/srv/app" refresh: true
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | `root` nie jest katalogiem. | Przekaż katalog główny projektu. |
| `E_INVALID_VALUE` | `sections`, `limit` lub `days` jest poza zakresem. | Użyj wartości wymienionych w sekcji Parametry. |
| `E_MISSING_PARAM` | Brakuje `root`. | Przekaż katalog główny. |

## Powiązane {#related}

`code.languages`, `code.symbols`, `code.outline`, `git.log`.
