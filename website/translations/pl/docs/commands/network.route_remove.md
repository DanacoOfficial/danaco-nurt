---
command: network.route_remove
group: administration
summary: Usuwa trasy pasujące do celu, następnego przeskoku, interfejsu i metryki.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# network.route_remove

## Składnia {#synopsis}

```text
network.route_remove <destination> [name: value]...
```

## Opis {#description}

`network.route_remove` usuwa trasy pasujące do `destination` oraz, gdy podano, do `gateway`, `interface` i `metric`: `ip route del` w systemie Linux, `route delete` w systemie macOS i `Remove-NetRoute` w systemie Windows. Wywołanie, które nie pasuje do żadnej trasy, kończy się błędem `E_NOT_FOUND`. Każda usunięta trasa jest zapisywana wraz z następnym przeskokiem, interfejsem i metryką, a `undo` dodaje ją ponownie poleceniem `network.route_add`.

Trasy i DNS są odczytywane przed zmianą. Raport zawiera stan przed zmianą i po niej (`via=<gateway> dev=<interface> metric=<n>` dla trasy, `servers=<list>;search=<list>` dla DNS), a `undo` go przywraca.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `destination` | `text` | wymagany | Sieć w postaci CIDR, adres hosta lub default. |
| `gateway` | `text` | brak | Następny przeskok. Tylko trasy przez ten następny przeskok. |
| `interface` | `text` | brak | Interfejs trasy. |
| `metric` | `int` | brak | Metryka trasy. |

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
| Warunki odwracalności | `undo` dodaje trasy, które zostały usunięte. |
| External use | `system_program`: `ip`, `route`, `netstat`, `resolvectl`, `networksetup`, `powershell`; Trasy i DNS są odczytywane i zmieniane za pomocą ip, route, netstat, resolvectl i networksetup oraz, w systemie Windows, za pomocą poleceń cmdlet NetTCPIP i DnsClient programu PowerShell. |

## Przykłady {#examples}

Usunięcie trasy do sieci.

```nurt example
network.route_remove "10.9.0.0/16" privilege: elevated
```

Usunięcie trasy, która używa jednego następnego przeskoku.

```nurt example
network.route_remove "10.9.0.0/16" gateway: "203.0.113.5" privilege: elevated
```

Usunięcie trasy jednego interfejsu.

```nurt example
network.route_remove "10.20.0.0/16" interface: "eth0" privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | Żadna trasa nie pasuje. | Odczytaj trasy systemu. |
| `E_INVALID_VALUE` | Wartość nie jest adresem, siecią ani nazwą interfejsu. | Popraw wartość. |
| `E_PRIVILEGE` | Program systemu odmówił zmiany z braku uprawnień. | Powtórz wywołanie z `privilege: elevated`. |

## Powiązane {#related}

`network.route_add`, `network.dns_set`.
