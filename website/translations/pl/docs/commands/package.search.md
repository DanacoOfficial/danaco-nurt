---
command: package.search
group: package
summary: Przeszukuje repozytoria pakietów.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# package.search

## Składnia {#synopsis}

```text
package.search <query> [name: value]...
```

## Opis {#description}

`package.search` wyszukuje pakiety w lokalnych indeksach repozytoriów menedżerów znalezionych w systemie. Rozpoczyna potok, nie przyjmuje wejścia i nie uruchamia żadnego programu. Indeksy są tymi, które menedżery przechowują po odświeżeniu; polecenie nigdy nie łączy się z repozytorium.

`query` jest dopasowywane do nazwy i opisu pakietu bez uwzględniania wielkości liter. Pakiet, który występuje w kilku repozytoriach lub architekturach, jest zgłaszany raz, z najwyższą wersją. `installed` ma wartość `true` dla pakietów, które są zainstalowane; dla nich `version` jest zainstalowaną wersją, a `available_version` jest ustawione, gdy repozytorium oferuje nowszą. Pakiet, którego nazwa jest równa `query`, występuje pierwszy, pozostałe następują w kolejności nazw.

Obsługiwane indeksy: listy apt (zwykłe, gzip, xz, bzip2, lz4 i zstd), metadane repozytoriów dnf i zypper, bazy synchronizacji pacman, pamięć podręczna API Homebrew, manifesty zasobników (bucket) Scoop oraz indeks źródła winget. Chocolatey nie przechowuje lokalnego indeksu i jest zgłaszany z ostrzeżeniem `no_local_index`. Brakujące lub uszkodzone indeksy są zgłaszane jako ostrzeżenia i pomijane.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `query` | `text` | wymagany | Wyszukiwane wyrażenia. |
| `manager` | `enum(auto\|apt\|dnf\|pacman\|zypper\|brew\|winget\|choco\|scoop)` | `auto` | Menedżer pakietów; auto wybiera menedżera systemu. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `package` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `package_manager` |

## Przykłady {#examples}

Wyszukanie pakietu według nazwy.

```nurt example
package.search "ripgrep"
```

Przeszukanie jednego menedżera z zachowaniem pakietów, które nie są zainstalowane.

```nurt example
package.search "editor" manager: apt |> filter installed == false
```

Wyświetlenie pierwszych dziesięciu dopasowań wraz z ich repozytoriami.

```nurt example
package.search "json" |> first 10 |> select name version source
```

Odpowiedź, która pomija rekordy, kończy się wskazówką „more: next” i zawiera
pole `continuation`; polecenie `next` zwraca kolejną część bez ponownego
uruchamiania polecenia.

```text
matched 330  shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_EXTERNAL_MISSING` | Nie znaleziono żadnego obsługiwanego menedżera pakietów albo podany nie jest zainstalowany. | Przekaż `manager` zainstalowanego menedżera. |
| `E_FORMAT` | Każdy indeks repozytorium podanego menedżera jest nieczytelny. | Odśwież indeksy za pomocą menedżera. |
| `E_LIMIT` | Dopasowania przekraczają limit rekordów. | Użyj bardziej szczegółowego zapytania. |

## Powiązane {#related}

`package.list`, `package.install`.
