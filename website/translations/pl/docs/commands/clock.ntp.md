---
command: clock.ntp
group: administration
summary: Włącza lub wyłącza synchronizację czasu przez sieć.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# clock.ntp

## Składnia {#synopsis}

```text
clock.ntp <state>
```

## Opis {#description}

`clock.ntp` włącza lub wyłącza synchronizację z czasem sieciowym: `timedatectl set-ntp` w systemie Linux, `systemsetup -setusingnetworktime` w systemie macOS i usługa `w32time` w systemie Windows. Stan, który system już ma, niczego nie zmienia. `undo` przywraca poprzedni stan.

Poprzednia wartość jest odczytywana przed zmianą i zapisywana jako cel `clock:ntp`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `state` | `enum(on\|off)` | wymagany | Czy synchronizować zegar z czasem sieciowym. |

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
| Warunki odwracalności | `undo` przywraca poprzedni stan. |
| External use | `system_program`: `timedatectl`, `systemsetup`, `tzutil`, `date`, `powershell`; Zegar jest odczytywany i ustawiany za pomocą timedatectl, systemsetup, tzutil, date oraz, w systemie Windows, PowerShell dla usługi czasu. |

## Przykłady {#examples}

Włączenie czasu sieciowego.

```nurt example
clock.ntp on privilege: elevated
```

Wyłączenie czasu sieciowego.

```nurt example
clock.ntp off privilege: elevated
```

Wyłączenie czasu sieciowego przed ręcznym ustawieniem zegara.

```nurt example
clock.ntp state: off privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | `state` nie jest ani `on`, ani `off`. | Wpisz `on` lub `off`. |
| `E_PRIVILEGE` | Program systemowy odmówił zmiany z powodu braku uprawnień. | Powtórz wywołanie z `privilege: elevated`. |

## Powiązane {#related}

`clock.timezone_set`, `clock.set`.
