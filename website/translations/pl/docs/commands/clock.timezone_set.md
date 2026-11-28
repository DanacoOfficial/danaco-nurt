---
command: clock.timezone_set
group: administration
summary: Ustawia strefę czasową systemu.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# clock.timezone_set

## Składnia {#synopsis}

```text
clock.timezone_set <zone>
```

## Opis {#description}

`clock.timezone_set` ustawia strefę czasową systemu: `timedatectl set-timezone` w systemie Linux, `systemsetup -settimezone` w systemie macOS i `tzutil /s` w systemie Windows, gdzie `zone` jest identyfikatorem systemu Windows, takim jak `W. Europe Standard Time`. Strefa, którą system już ma, niczego nie zmienia. `undo` ustawia poprzednią strefę.

Poprzednia wartość jest odczytywana przed zmianą i zapisywana jako cel `clock:timezone`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `zone` | `text` | wymagany | Nazwa strefy czasowej, na przykład Europe/Warsaw; w systemie Windows identyfikator, taki jak W. Europe Standard Time. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `external` |
| Odwracalność | `reversible` |
| Implementacja | `builtin` |
| Warunki odwracalności | `undo` ustawia poprzednią strefę czasową. |
| External use | `system_program`: `timedatectl`, `systemsetup`, `tzutil`, `date`, `powershell`; Zegar jest odczytywany i ustawiany za pomocą timedatectl, systemsetup, tzutil, date oraz, w systemie Windows, PowerShell dla usługi czasu. |

## Przykłady {#examples}

Ustawienie strefy czasowej systemu.

```nurt example
clock.timezone_set "Europe/Warsaw" privilege: elevated
```

Użycie UTC.

```nurt example
clock.timezone_set "UTC" privilege: elevated
```

Ustawienie strefy czasowej systemu Windows.

```nurt example
clock.timezone_set "W. Europe Standard Time" privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | `zone` nie jest nazwą strefy czasowej. | Wpisz nazwę z bazy stref czasowych. |
| `E_PRIVILEGE` | Program systemowy odmówił zmiany z powodu braku uprawnień. | Powtórz wywołanie z `privilege: elevated`. |
| `E_STAGE_FAILED` | System nie zna tej strefy. | Wyświetl strefy za pomocą `timedatectl list-timezones`. |

## Powiązane {#related}

`clock.ntp`, `clock.set`.
