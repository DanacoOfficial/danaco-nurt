---
command: program.find
group: program
summary: Lokalizuje programy w PATH w kolejności, w jakiej rozwiązuje je system.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# program.find

## Składnia {#synopsis}

```text
program.find <name>... [name: value]...
```

## Opis {#description}

`program.find` rozwiązuje każdą nazwę w katalogach `PATH` w kolejności używanej przez system operacyjny i wyświetla jeden rekord `program_location` dla każdej nazwy, w kolejności argumentów. Rozpoczyna potok i nie przyjmuje wejścia. Powtórzone nazwy są zgłaszane jeden raz.

Nazwa, która się rozwiązuje, daje `found: true`, pierwszy pasujący plik w `path` oraz, z `all`, każde dopasowanie w `all_paths`. Nazwa, która się nie rozwiązuje, daje rekord z `found: false` i bez ścieżki; polecenie nie kończy się błędem. W systemie Windows rozszerzenia z `PATHEXT` są próbowane po kolei, a nazwy są porównywane bez uwzględniania wielkości liter; w systemie macOS nazwy są porównywane bez uwzględniania wielkości liter; w systemie Linux i innych systemach Unix porównanie jest dokładne i dopasowywane są tylko pliki z uprawnieniem do wykonania. Dowiązanie symboliczne w `PATH` wskazujące na inny program w `PATH` jest zgłaszane z polem `alias_of`.

Nazwa zawierająca separator ścieżki, taka jak `./tool` lub `bin/tool`, nie jest wyszukiwana w `PATH`; plik jest sprawdzany w podanej postaci, a nazwa względna względem katalogu bazowego żądania.

`program.find` czyta tylko katalogi `PATH`; nie buduje indeksu programów, chyba że `versions` ma wartość `true`. Z `versions` wersja pochodzi z indeksu programów: z bazy pakietów, wpisu aplikacji lub katalogu, a `version_source` wskazuje, które z `package`, `app` lub `catalog` ją podało. Żaden program nie jest uruchamiany; zlokalizowany program bez wersji w tych źródłach jej nie ma.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `list<text>` | wymagany | Nazwy programów. |
| `versions` | `bool` | `false` | Odczytuje wersję każdego zlokalizowanego programu. |
| `all` | `bool` | `false` | Wyświetla każde dopasowanie w all_paths. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none` | `program_location` |

Pola `program_location`: `name`, `found`, `path`, `all_paths`, `version`, `source` oraz dodatkowe pola `alias_of` i `version_source`, gdy mają zastosowanie.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Zlokalizowanie dwóch programów.

```nurt example
program.find "git" "rg"
```

```text expected
name  found  path
git   true   /usr/bin/git
rg    true   /usr/local/bin/rg
```

Zgłoszenie nazw, które nie są zainstalowane.

```nurt example
program.find "ffmpeg" "gs" "magick" |> filter found == false |> select name
```

Wyświetlenie zainstalowanej wersji każdego programu wraz z jej źródłem.

```nurt example
program.find "python3" "node" versions: true |> select name version version_source
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | Nie podano nazwy. | Podaj co najmniej jedną nazwę programu. |
| `E_CANCELLED` | Wykonanie zostało anulowane w trakcie rozwiązywania nazw. | Uruchom polecenie ponownie. |

## Powiązane {#related}

`program.search`, `program.describe`, `program.catalog`.
