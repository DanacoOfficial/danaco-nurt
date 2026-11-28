---
command: program.catalog
group: program
summary: Przegląda programy według kategorii: kategorie, następnie podkategorie lub programy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# program.catalog

## Składnia {#synopsis}

```text
program.catalog [<category>] [name: value]...
```

## Opis {#description}

`program.catalog` wyświetla programy z indeksu według kategorii. Rozpoczyna potok i nie przyjmuje wejścia.

Bez `category` wynikiem jest jeden rekord `program_category` dla każdej kategorii zawierającej co najmniej jeden program, uporządkowany malejąco według liczby programów, a następnie według identyfikatora. Kategorie pochodzą z wbudowanej taksonomii i z katalogów zewnętrznych; kategoria katalogu o identyfikatorze kategorii wbudowanej lub deklarująca ją w `same_as` jest tą samą kategorią. Programy komputera są umieszczane według wpisu pulpitu, sekcji lub grupy pakietu, sekcji strony podręcznika, a w ich braku według klasyfikatora słów kluczowych; `category_source` na każdej karcie wskazuje, który z nich zastosowano.

Z `category` i kategorią podzieloną na podkategorie wynikiem jest jeden rekord `program_category` dla każdej podkategorii zawierającej programy, w kolejności katalogu, a po nich podkategoria `_other` dla programów spoza wszystkich podkategorii. Z kategorią bez podkategorii lub z `subcategory` wynikiem są rekordy `program_card` jej programów uporządkowane według nazwy. `subcategory` przyjmuje identyfikator lub alias podkategorii oraz `_other`.

Z `query` programy kategorii są szeregowane jak w `program.search` i wyświetlanych jest najwyżej 30; `subcategory` jest wtedy ignorowane.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `category` | `text` | none | Kategoria do otwarcia; bez niej wynik wyświetla wszystkie kategorie. |
| `subcategory` | `text` | none | Podkategoria do otwarcia. |
| `query` | `text` | none | Słowa kluczowe zawężające wyświetlane programy. |
| `source` | `text` | none | Tylko programy z tego źródła. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| default | `none` | `program_category`, `program_card` |

Rekordy `program_category` zawierają dodatkowe pola `level` (`category` lub `subcategory`), `parent`, `summary`, `aliases`, `sources` i `counts_by_source`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie kategorii programów komputera.

```nurt example
program.catalog
```

```text expected
name      count  title
system    412    System and administration
code      138    Development
network   57     Network
```

Otwarcie jednej kategorii i wyświetlenie jej programów.

```nurt example
program.catalog "archives" |> select name summary
```

Uszeregowanie programów kategorii z jednego katalogu.

```nurt example
program.catalog "video" query: "subtitles" source: "catalog" |> first 5
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | Kategoria jest nieznana lub nie zawiera programów; `candidates` wyświetla najbliższe kategorie. | Użyj kategorii z listy. |
| `E_ARG` | Podano `subcategory` bez `category`, kategoria nie ma podkategorii, podkategoria jest nieznana albo zapytanie jest krótsze niż 2 lub dłuższe niż 200 znaków. | Podaj `category`, pomiń `subcategory` lub użyj podkategorii z listy. |
| `E_CATALOG` | `source` wskazuje katalog programów, który nie jest skonfigurowany lub nie może być odczytany. | Sprawdź `programs.catalogs`. |

## Powiązane {#related}

`program.search`, `program.describe`, `program.find`.
