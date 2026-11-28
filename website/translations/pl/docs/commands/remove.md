---
command: remove
group: core
summary: Usuwa pliki, katalogi, klucze i wartości rejestru, wiersze, dopasowania, elementy dokumentu, węzły składni lub zakończone zadania.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# remove

## Składnia {#synopsis}

```text
remove <path>... [name: value]...
entry|registry_value|line|match|item|node|job |> remove [name: value]...
```

## Opis {#description}

`remove` usuwa pliki, dowiązania symboliczne, punkty połączenia i katalogi podane przez ścieżkę lub rekordy `entry`. Katalog jest usuwany wraz z całą zawartością jako jeden cel, bez osobnego potwierdzenia. Dowiązanie jest usuwane jako dowiązanie; jego cel pozostaje. Ścieżka wewnątrz innej usuwanej ścieżki jest usuwana jeden raz, wraz z obejmującym ją katalogiem.

W ścieżce zapisanej w programie `..` występujące po dowiązaniu symbolicznym jest odrzucane błędem `E_INVALID_VALUE` z poprawką w postaci ścieżki bez `..`: system kontynuowałby od celu dowiązania, a język od katalogu, który je zawiera, a `remove` nie może usunąć innego wpisu niż zapisany. Ta sama reguła obowiązuje dla `move`, miejsca docelowego `copy`, `write`, `edit`, `insert`, `create`, `replace`, `attributes`, `convert` i `compress`; polecenia, które tylko czytają, zachowują leksykalne znaczenie `..`.

`on_missing` rozstrzyga, co dzieje się ze ścieżką, która nie istnieje: `skip` liczy ją jako `missing` i kontynuuje, `error` kończy się błędem `E_NOT_FOUND` przed jakąkolwiek zmianą. Rekord `entry` z `exists: false`, taki jaki tworzy `info`, jest brakującą ścieżką.

Rekordy `line`, takie jak tworzą `lines` i `read`, usuwają całe wiersze wraz z ich zakończeniami wierszy; rekordy `match`, takie jak tworzy `search`, usuwają dopasowany tekst w jego wierszu i kolumnie. Wszystkie wiersze i dopasowania jednego pliku są usuwane jednym zapisem, który zachowuje kodowanie, znacznik kolejności bajtów i zakończenia wierszy pliku. Pliku nie można usunąć i edytować w tym samym wywołaniu. Rekordy `item`, `node` i `job` są usuwane przez etapy poleceń dokumentów, składni i zadań.

`check_references: true` przeszukuje pliki poniżej katalogu bazowego lub poniżej katalogu pierwszej usuwanej ścieżki w poszukiwaniu wystąpień całych słów usuniętych nazw oraz nazw bez ostatniego rozszerzenia i zgłasza je jako `references_left` z maksymalnie 20 lokalizacjami. Ukryte wpisy, kontrola wersji, katalogi zależności, kompilacji i pamięci podręcznej, pliki binarne oraz pliki większe niż 1 MiB nie są przeszukiwane.

Rekord z `etag` musi nadal mieć ten znacznik jednostki; zmieniony wpis lub plik kończy się błędem `E_STALE` przed jakąkolwiek zmianą. Gdy nic nie zostanie usunięte, wynik zgłasza `nothing_matched`.

| Godny uwagi kod | Znaczenie |
|---|---|
| `references_left` | inne pliki nadal wymieniają usunięty wpis; `detail` wyświetla lokalizacje |
| `nothing_matched` | nic nie zostało usunięte |

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `path` | `list<path>` | wymagany | Ścieżki do usunięcia w postaci źródłowej; katalogi są usuwane rekurencyjnie. |
| `on_missing` | `enum(skip\|error)` | `skip` | Zachowanie, gdy ścieżka nie istnieje. |
| `check_references` | `bool` | `false` | Zgłasza odwołania do usuniętych nazw pozostałe po zmianie. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| `source` | `none` | `effect` |
| `stage` | `entry`, `registry_value`, `line`, `match`, `item`, `node`, `job` | `effect` |

Rekord `effect` ma jedną zmianę `change` z akcją `delete` dla każdego usuniętego wpisu; katalog usunięty wraz z zawartością ma flagę `recursive`. Plik edytowany przez rekordy `line` lub `match` ma jedną zmianę `change` z akcją `modify` i liczbą usuniętych wierszy lub dopasowań. Podsumowanie `summary` zlicza `removed`, `files`, `directories`, `links`, `bytes`, `missing`, `lines` i `matches`.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |
| Warunki odwracalności | Usunięte pliki, katalogi i zawartość są przechowywane w dzienniku. |

| Akcja | Skutek | Odwracalność | Uwaga |
|---|---|---|---|
| `job` | `local` | `irreversible` | Usunięcie zadania kasuje jego rekord i pliki wyjściowe bez kopii w dzienniku. |

## Przykłady {#examples}

Usunięcie pliku:

```nurt example
remove "/home/user/build.log"
```

```text expected
remove: 1 target, files 1
```

Usunięcie drzewa katalogów i pominięcie ścieżki, która nie istnieje:

```nurt example
remove "/home/user/app/cache" "/home/user/app/old.log"
```

Usunięcie dwóch wierszy z pliku:

```nurt example
lines "/home/user/notes.txt" 2..3 |> remove
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | postać źródłowa nie ma ścieżki | podaj `path` |
| `E_NOT_FOUND` | ścieżka nie istnieje przy `on_missing: error` | sprawdź ścieżkę lub użyj `on_missing: skip` |
| `E_STALE` | wpis lub plik zmienił się od odczytu albo wiersz lub dopasowanie nie znajduje się już tam, gdzie wskazuje rekord | odczytaj lub przeszukaj plik ponownie |
| `E_INVALID_VALUE` | plik jest usuwany i edytowany w tym samym wywołaniu albo ścieżka ma `..` po dowiązaniu symbolicznym | usuń plik lub jego wiersze, nie oba naraz, albo zapisz ścieżkę bez `..` |
| `E_TYPE` | rekord rodzaju, którego `remove` nie przyjmuje, albo rekord `line` lub `match` bez pól numerów | przekaż rekordy `list`, `find`, `lines`, `read` lub `search` |
| `E_IN_USE` | Windows: wpis jest otwarty w innym procesie; błąd wyświetla procesy | zamknij wpis w tym procesie |
| `E_PATH_RELATIVE` | ścieżka względna bez katalogu bazowego | podaj ścieżkę bezwzględną lub ustaw `base` |

## Powiązane {#related}

`move`, `rename`, `copy`, `lines`, `search`, `find`, `undo`.
