---
command: modules.load
group: administration
summary: Ładuje moduł jądra.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# modules.load

## Składnia {#synopsis}

```text
modules.load <name> [name: value]...
```

## Opis {#description}

`modules.load` ładuje moduł jądra za pomocą `modprobe` w systemie Linux; `params` są przekazywane jako `name=value`. Moduł, który jest już załadowany, ma wynik `unchanged`. O tym rozstrzyga `lsmod`; znaki `-` i `_` w nazwie modułu są tym samym znakiem.

Uprawnienia: polecenie przyjmuje `privilege`; wpisz `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który kończy się niepowodzeniem z braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je po podniesieniu uprawnień. Wpis z podniesionymi uprawnieniami jest cofany przez ten sam proces z podniesionymi uprawnieniami.

Cofnięcie (`undo`) uruchamia `modules.unload`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text` | wymagany | Nazwa modułu. |
| `params` | `list<text>` | brak | Parametry modułu jako name=value. |

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
| Warunki odwracalności | Cofnięcie wyładowuje moduł. |
| Systemy operacyjne | `linux` |
| External use | `system_program`: `modprobe`, `lsmod`; Moduły są wyświetlane za pomocą lsmod i ładowane za pomocą modprobe. |

## Przykłady {#examples}

Załadowanie modułu.

```nurt example
modules.load "br_netfilter" privilege: elevated
```

Załadowanie modułu z parametrem.

```nurt example
modules.load "nf_conntrack" params: ["hashsize=65536"] privilege: elevated
```

Uruchomienie ładowania z podniesionymi uprawnieniami; dodanie parametru wywołania `preview: true` najpierw pokazuje skutek.

```nurt example
modules.load "overlay" privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_OS_ONLY` | System nie jest systemem Linux. | Użyj narzędzi systemu. |
| `E_STAGE_FAILED` | Moduł nie istnieje. | Sprawdź nazwę. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wymieniony w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`modules.unload`, `sysctl.set`.
