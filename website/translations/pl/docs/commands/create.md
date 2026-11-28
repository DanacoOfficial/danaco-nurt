---
command: create
group: core
summary: Tworzy pliki, katalogi, dowiązania, punkty połączeń, potoki nazwane, węzły urządzeń lub wpisy tymczasowe.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# create

## Składnia {#synopsis}

```text
create <path>... type: <enum(file|directory|link|junction|fifo|device)> [name: value]...
```

## Opis {#description}

`create` tworzy jeden wpis na każdą ścieżkę: pusty plik, katalog, dowiązanie
symboliczne lub twarde, punkt połączenia (junction), potok nazwany albo węzeł
urządzenia. Parametr `type` nie ma wartości domyślnej. Brakujące katalogi
nadrzędne są tworzone, chyba że `parents` ma wartość `false`; każdy z nich jest
osobnym celem, a katalog wymieniony dwa razy, jako ścieżka i jako katalog
nadrzędny, jest tworzony jeden raz.

Istniejący wpis żądanego rodzaju jest pozostawiany bez zmian przy
`on_exists: skip` i zgłaszany jako `unchanged`, kończy się błędem `E_EXISTS`
przy `on_exists: error`, a przy `on_exists: touch` otrzymuje bieżący czas jako
czas dostępu i modyfikacji. Istniejący wpis innego rodzaju zawsze kończy się
błędem `E_EXISTS`.

Dowiązanie symboliczne przechowuje `target` tak, jak go zapisano; cel względny
jest liczony względem katalogu dowiązania, a cel nie musi istnieć. `hard: true`
tworzy dowiązanie twarde do istniejącego pliku `target`. Punkt połączenia
wymaga istniejącego katalogu jako celu i istnieje wyłącznie w systemie Windows.

Przy `temporary: true` każda ścieżka `path` jest katalogiem, a `create` tworzy
w każdym z nich jeden plik lub katalog o unikalnej nazwie zaczynającej się od
`nurt-`; bez `path` w katalogu tymczasowym hosta. Rekordy `change` wyniku
zawierają nowe ścieżki.

`type: fifo` tworzy potok nazwany (FIFO) z bitami uprawnień z `mode`,
zapisanymi w systemie ósemkowym, na przykład `mode: "0660"` (bez `mode`
wartość domyślna `0666` jest zmniejszana o umask; przy `mode` bity są
ustawiane dokładnie, bez śledzenia dowiązania). `type: device` tworzy węzeł
urządzenia i wymaga `kind: char` lub `kind: block`, `major` i `minor`; `mode`
działa tak jak dla potoku. Oba rodzaje istnieją tylko w systemach Linux
i macOS; w systemie Windows kończą się błędem `E_OS_ONLY`, a dla potoku
komunikat wskazuje potoki nazwane systemu, czyli obiekty przestrzeni nazw
`\\.\pipe\` tworzone przez program, który je obsługuje. Węzeł urządzenia
wymaga `privilege: elevated`; bez tego system odmawia z błędem `E_PRIVILEGE`.
Dokładny `mode` jest ustawiany przez `/proc/self/fd` w systemie Linux; bez
`/proc` polecenie kończy się niepowodzeniem i usuwa nowy węzeł. Oba rodzaje są
zapisywane w dzienniku jako utworzone pliki, a `undo` je usuwa. `info` i `list`
zgłaszają taki wpis z typem `fifo` lub `device`; węzeł urządzenia zgłasza także
`major` i `minor`. Parametrów `kind`, `major`, `minor` i `mode` nie można
łączyć z innymi typami, a `temporary` nie można łączyć z `fifo` ani `device`.

W systemie Windows dowiązanie symboliczne wymaga uprawnienia do tworzenia
dowiązań symbolicznych albo trybu programisty; bez tego polecenie kończy się
błędem `E_PRIVILEGE`, a dla katalogów alternatywą jest punkt połączenia.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `type` | `enum(file\|directory\|link\|junction\|fifo\|device)` | wymagany | Rodzaj wpisu; nie ma wartości domyślnej. fifo (potok nazwany) i device (węzeł urządzenia) istnieją w systemach Linux i macOS; device wymaga podwyższonych uprawnień. |
| `path` | `list<path>` | brak | Ścieżki do utworzenia; przy temporary katalog nowego wpisu. |
| `target` | `text` | brak | Cel dowiązania lub punktu połączenia. |
| `hard` | `bool` | `false` | Tworzy dowiązanie twarde zamiast symbolicznego. |
| `temporary` | `bool` | `false` | Tworzy wpis o unikalnej nazwie w katalogu tymczasowym. |
| `parents` | `bool` | `true` | Tworzy brakujące katalogi nadrzędne. |
| `on_exists` | `enum(skip\|error\|touch)` | `skip` | Zachowanie, gdy wpis istnieje; touch aktualizuje jego czas modyfikacji. |
| `kind` | `enum(char\|block)` | brak | Rodzaj węzła urządzenia dla typu device. |
| `major` | `int` | brak | Główny numer urządzenia dla typu device. |
| `minor` | `int` | brak | Poboczny numer urządzenia dla typu device. |
| `mode` | `text` | brak | Bity uprawnień potoku fifo lub węzła urządzenia w zapisie ósemkowym, na przykład "0660"; wartością domyślną jest 0666 zmniejszone o umask. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `local` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Utworzenie katalogu i jego brakującego katalogu nadrzędnego:

```nurt example
create "/home/user/app/build/cache" type: directory
```

Aktualizacja czasu modyfikacji istniejącego pliku:

```nurt example
create "/home/user/notes.txt" type: file on_exists: touch
```

```json fs-before
{ "format": "nurt-fs-snapshot/1", "entries": [
  { "path": "/home/user/notes.txt", "type": "file", "content_utf8": "keep\n" }
] }
```

Skierowanie dowiązania na katalog wydania:

```nurt example
create "/home/user/app/current" type: link target: "releases/v2"
```

Utworzenie pustego pliku o unikalnej nazwie:

```nurt example
create "/home/user/tmp" type: file temporary: true
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_MISSING_PARAM` | brakuje `type`, brakuje `path` bez `temporary`, dowiązanie nie ma `target` albo urządzeniu brakuje `kind`, `major` lub `minor` | podaj brakujący parametr |
| `E_EXISTS` | wpis istnieje przy `on_exists: error` albo istnieje jako inny rodzaj | wybierz inną ścieżkę lub `on_exists: skip` |
| `E_PARAM_CONFLICT` | `target` lub `hard` z typem innym niż `link`, `kind`, `major` lub `minor` z typem innym niż `device`, `mode` z typem innym niż `fifo` lub `device`, `temporary` z dowiązaniem, potokiem fifo lub urządzeniem albo ścieżka podana dwa razy | usuń parametr |
| `E_INVALID_VALUE` | `major` lub `minor` przekracza numer urządzenia systemu (Linux: major do 4095, minor do 1048575; macOS: 255 i 16777215) albo `mode` nie jest ósemkowymi bitami uprawnień | podaj poprawną wartość |
| `E_NOT_FOUND` | brakuje katalogu nadrzędnego, a `parents` ma wartość `false`, albo brakuje celu dowiązania twardego lub punktu połączenia | ustaw `parents: true` albo utwórz cel |
| `E_OS_ONLY` | `type: junction` poza systemem Windows, `type: fifo` lub `type: device` w systemie Windows | użyj `type: link`; w systemie Windows utwórz potok nazwany w programie, który go obsługuje |
| `E_PRIVILEGE` | system Windows odmawia utworzenia dowiązania symbolicznego albo węzeł urządzenia jest tworzony bez `privilege: elevated` | włącz tryb programisty, użyj punktu połączenia albo dodaj `privilege: elevated` |

## Powiązane {#related}

`write`, `copy`, `move`, `remove`, `attributes`, `list`.
