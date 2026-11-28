---
command: modules.unload
group: administration
summary: Wyładowuje moduł jądra.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# modules.unload

## Składnia {#synopsis}

```text
modules.unload <name>
```

## Opis {#description}

`modules.unload` wyładowuje moduł jądra za pomocą `modprobe -r` w systemie Linux. Moduł, który nie jest załadowany, ma wynik `unchanged`.

Uprawnienia: polecenie przyjmuje `privilege`; wpisz `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który kończy się niepowodzeniem z braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je po podniesieniu uprawnień. Wpis z podniesionymi uprawnieniami jest cofany przez ten sam proces z podniesionymi uprawnieniami.

Cofnięcie (`undo`) uruchamia `modules.load`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text` | wymagany | Nazwa modułu. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `external` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `system_program` |
| Warunki odwracalności | Cofnięcie ładuje moduł. |
| Systemy operacyjne | `linux` |
| External use | `system_program`: `modprobe`, `lsmod`; Moduły są wyświetlane za pomocą lsmod i wyładowywane za pomocą modprobe -r. |

## Przykłady {#examples}

Wyładowanie modułu.

```nurt example
modules.unload "dummy" privilege: elevated
```

Wyładowanie i pokazanie skutku.

```nurt example
modules.unload "dummy" privilege: elevated |> select target
```

Uruchomienie wyładowania z podniesionymi uprawnieniami; dodanie parametru wywołania `preview: true` najpierw pokazuje skutek.

```nurt example
modules.unload "dummy" privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_OS_ONLY` | System nie jest systemem Linux. | Użyj narzędzi systemu. |
| `E_STAGE_FAILED` | Moduł jest używany. | Najpierw zatrzymaj jego użytkowników. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wymieniony w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`modules.load`.
