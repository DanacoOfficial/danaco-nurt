---
command: program.search
group: program
summary: Przeszukuje zainstalowane programy, bazy pakietów, aplikacje systemowe i skonfigurowane katalogi programów.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# program.search

## Składnia {#synopsis}

```text
program.search <query> [name: value]...
```

## Opis {#description}

`program.search` szereguje karty indeksu programów względem zapytania i wyświetla najlepsze rekordy `program_card` wraz z ich `score`, od najwyższego. Rozpoczyna potok i nie przyjmuje wejścia.

Indeks łączy programy z `PATH`, bazy pakietów (dpkg, rpm, pacman, Homebrew, winget, Scoop, Chocolatey), zarejestrowane aplikacje (wpisy pulpitu, wpisy odinstalowania i ścieżki aplikacji w systemie Windows, pakiety aplikacji w systemie macOS), wiersze `NAME` stron podręcznika oraz katalogi zewnętrzne z `programs.catalogs`. Działa bez konfiguracji na programach komputera. Pierwsze wywołanie procesu buduje indeks; kolejne wywołania używają go ponownie, dopóki pliki każdego dostawcy są niezmienione. Z ustawionym `programs.index_dir` każdy dostawca jest dodatkowo przechowywany w pliku pamięci podręcznej, a nowy proces startuje z niego.

Słowa zapytania są porównywane po ujednoliceniu: małe litery, dekompozycja zgodnościowa i brak znaków diakrytycznych, więc zapytanie wpisane bez znaków diakrytycznych pasuje do tekstu z nimi. Częste spójniki i przyimki są ignorowane, gdy pozostają inne słowa. Każde słowo daje 100 punktów, gdy jest równe nazwie programu, 40, gdy nazwa zaczyna się od niego, 20, gdy nazwa je zawiera, 8 w sytuacjach użycia, 5 w streszczeniu, 5 w kategorii i 3 w opisie przewodnika; zapytanie, którego każde słowo zostało znalezione gdziekolwiek, zyskuje 15. Słowo bez żadnego dopasowania jest ponawiane z połową wagi przez wbudowaną tabelę synonimów, synonimy katalogów i rdzeń prefiksowy słów o długości sześciu znaków lub więcej. Program, którego wpis `not_for` obejmuje każde słowo zapytania, traci 30 punktów i otrzymuje pole `instead` z programem do użycia. Równe wyniki są porządkowane według priorytetu dostawcy, a następnie według nazwy. Wyniki bez punktów są odrzucane.

Lista zapytań daje jedną uszeregowaną listę dla każdego zapytania; każdy rekord ma wtedy pole `query`.

`kind` zachowuje tylko programy, aplikacje lub samodzielne przewodniki katalogów; wartość domyślna `all` zachowuje każdy rodzaj. `installed: true` zachowuje programy istniejące na tym komputerze, a `installed: false` wpisy, które nie istnieją; bez niego wyświetlane są oba.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `query` | `text \| list<text>` | wymagany | Zadanie, temat lub nazwa programu albo kilka zapytań, każde z osobną listą wyników. |
| `category` | `text` | none | Tylko programy z tej kategorii. |
| `source` | `list<enum(path\|package\|app\|catalog)>` | `[path, package, app, catalog]` | Przeszukiwane źródła. |
| `limit` | `int` | `15` | Maksymalna liczba programów, najlepsze dopasowanie najpierw. |
| `kind` | `enum(all\|programs\|apps\|guides)` | `all` | Rodzaj wyników. |
| `installed` | `bool` | none | Tylko zainstalowane programy (true) lub tylko wpisy katalogu (false). |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none` | `program_card` |

Każdy rekord zawiera pola `program_card` oraz dodatkowe pola `score`, `kind` (`program`, `app` lub `guide`), `has_guide`, `category_source` i, gdy mają zastosowanie, `query`, `instead`, `guide_name`, `guide_shared_with`, `overview`, `alternatives`, `catalog_path`, `manager`, `app_id` i `language`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

Dostawcy, których nie można odczytać, oraz dostawcy przerwani przez budżet czasu indeksu są zgłaszani jako ostrzeżenia (`source_unavailable`, `index_partial`); polecenie nadal zwraca wyniki pozostałych dostawców.

## Przykłady {#examples}

Wyszukanie programów do zadania.

```nurt example
program.search "convert video" limit: 5
```

```text expected
name     category  summary
ffmpeg   video     Converts and streams audio and video.
handbrake video    Transcodes video files.
```

Przeszukanie tylko zainstalowanych pakietów i ścieżki wyszukiwania.

```nurt example
program.search "pdf merge" source: [path, package] |> select name path score
```

Wykonanie dwóch zapytań naraz i zachowanie najlepszego dopasowania każdego z nich.

```nurt example
program.search ["compress files", "edit images"] limit: 1 |> select query name
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Nie podano zapytania. | Podaj zapytanie. |
| `E_ARG` | Zapytanie jest krótsze niż 2 lub dłuższe niż 200 znaków. | Skróć lub wydłuż zapytanie. |
| `E_INVALID_VALUE` | `limit` jest poza zakresem od 1 do 60, podano więcej niż 8 zapytań albo kategoria jest nieznana; `candidates` wyświetla najbliższe kategorie. | Popraw wartość lub wyświetl kategorie poleceniem `program.catalog`. |
| `E_CATALOG` | `source` wskazuje katalog programów, który nie jest skonfigurowany lub nie może być odczytany. | Sprawdź `programs.catalogs`. |

## Powiązane {#related}

`program.describe`, `program.catalog`, `program.find`.
