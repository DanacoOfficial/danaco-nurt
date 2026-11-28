---
command: program.describe
group: program
summary: Opisuje program wraz z jego wersją, źródłem, opisem i przewodnikiem użycia.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# program.describe

## Składnia {#synopsis}

```text
program.describe <name> [name: value]...
```

## Opis {#description}

`program.describe` wyświetla jeden rekord `program_card` z pełnym opisem programu, aplikacji lub przewodnika użycia. Rozpoczyna potok i nie przyjmuje wejścia.

Nazwa jest wyszukiwana w indeksie programów: najpierw jako nazwa karty, następnie jako alias (dowiązanie symboliczne w `PATH`, alternatywa Debiana, shim Scoop lub Chocolatey, alias wykonania aplikacji albo alias zadeklarowany przez katalog), a potem jako nazwa samodzielnego przewodnika katalogu. Dla aliasu rekord opisuje program główny; pole `requested` zawiera podaną nazwę, a `alias_kind` rodzaj aliasu. W systemach Windows i macOS nazwy są porównywane bez uwzględniania wielkości liter.

Rekord łączy dane od każdego dostawcy znającego program: ścieżkę i alternatywy z `PATH`, wersję, stronę domową, licencję i opis pakietu z bazy pakietów, nazwę i identyfikator aplikacji z wpisu aplikacji, wiersz `NAME` strony podręcznika w polu `man` oraz opis, sytuacje użycia, zadania, do których program nie jest przeznaczony, przykład, pułapki i przewodnik użycia z katalogów zewnętrznych. `guide_sections` wyświetla nagłówki przewodnika; `etag` identyfikuje zawartość rekordu, a `guide_etag` zawartość przewodnika. Programy współdzielące jeden przewodnik wymieniają się wzajemnie w `guide_shared_with`.

Z listą nazw polecenie wyświetla jeden rekord dla każdej istniejącej nazwy, a pozostałe zgłasza jako ostrzeżenia; kończy się błędem `E_NOT_FOUND` tylko wtedy, gdy żadna nazwa nie istnieje. `section` wybiera jedną sekcję przewodnika według nagłówka, `query` sekcje przewodnika pasujące do jego słów, a `include_guide: false` pomija przewodnik. Gdy `unchanged_since` jest równe `etag` rekordu, rekord zawiera `unchanged: true` i nic więcej z opisu. `source` ogranicza wyszukiwanie do wymienionych źródeł, nazwanych jak w `program.catalog`.

Tekst pomocy programu nigdy nie jest odczytywany samodzielnie: `program.describe` nie uruchamia żadnego programu, chyba że podano `help: true`. Program jest wtedy uruchamiany jednokrotnie z `--help` w limicie czasu `programs.help_timeout`, a jego wyjście jest zwracane w polu `help`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text \| list<text>` | wymagany | Nazwa programu lub do 12 nazw. |
| `include_guide` | `bool` | `true` | Dołącza przewodnik użycia, gdy jest dostępny. |
| `section` | `text` | none | Zwraca tylko sekcję przewodnika o tym nagłówku. |
| `query` | `text` | none | Zwraca tylko sekcje przewodnika pasujące do tych słów kluczowych. |
| `unchanged_since` | `text` | none | Etag wcześniejszego rekordu; niezmieniony program zwraca tylko unchanged. |
| `source` | `text \| list<text>` | none | Tylko programy z tych źródeł. |
| `help` | `bool` | `false` | Uruchamia program jednokrotnie z --help i zwraca jego wyjście w polu help. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none` | `program_card` |

Rekord zawiera pola `program_card` oraz, gdy mają zastosowanie, `kind`, `etag`, `requested`, `alias_kind`, `description`, `when_to_use`, `not_for`, `example`, `pitfalls`, `man`, `guide`, `guide_sections`, `guide_files`, `guide_etag` i `guide_shared_with`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |
| External use | `user_program`; używane, gdy podano `help`; Uruchamia program jednokrotnie z --help, w limicie czasu, aby odczytać jego tekst pomocy. |

## Przykłady {#examples}

Opis zainstalowanego programu.

```nurt example
program.describe "tar"
```

```text expected
name  category  version  path          summary
tar   archives  1.35     /usr/bin/tar  An archiving utility.
```

Opis programu, którego przewodnik został już odczytany w sesji.

```nurt example
program.describe "ffmpeg" include_guide: false |> select name guide_sections guide_etag
```

Rozwiązanie aliasu do programu głównego.

```nurt example
program.describe "vi" |> select name requested alias_kind path
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Nie podano nazwy. | Podaj nazwę programu. |
| `E_INVALID_VALUE` | Nazwa jest dłuższa niż 120 znaków. | Podaj nazwę programu bez argumentów. |
| `E_NOT_FOUND` | Żaden program, aplikacja ani przewodnik nie ma tej nazwy; `candidates` wyświetla do ośmiu podobnych nazw. | Użyj kandydata lub znajdź program poleceniem `program.search`. |

## Powiązane {#related}

`program.search`, `program.find`, `program.catalog`.
