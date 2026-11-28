---
command: network.route_add
group: administration
summary: Dodaje trasę do tablicy tras.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# network.route_add

## Składnia {#synopsis}

```text
network.route_add <destination> [name: value]...
```

## Opis {#description}

`network.route_add` dodaje trasę do tablicy tras: `ip route add` w systemie Linux, `route add` w systemie macOS i `New-NetRoute` w systemie Windows. `destination` jest siecią w postaci CIDR, adresem hosta albo `default`. Trasa wymaga `gateway` lub `interface`. Gdy tablica już zawiera trasę o tym samym celu, następnym przeskoku, interfejsie i metryce, nic nie jest zmieniane, a raport podaje `unchanged`. Cofnięcie (`undo`) usuwa trasę poleceniem `network.route_remove`.

Trasy i DNS są odczytywane przed zmianą. Raport zawiera stan przed zmianą i po niej (`via=<gateway> dev=<interface> metric=<n>` dla trasy, `servers=<list>;search=<list>` dla DNS), a `undo` go przywraca.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `destination` | `text` | wymagany | Sieć w postaci CIDR, adres hosta lub default. |
| `gateway` | `text` | brak | Następny przeskok. Wymagany, chyba że podano interface. |
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
| Warunki odwracalności | `undo` usuwa trasę; trasa, która już istniała, pozostaje nietknięta. |
| External use | `system_program`: `ip`, `route`, `netstat`, `resolvectl`, `networksetup`, `powershell`; Trasy i DNS są odczytywane i zmieniane za pomocą ip, route, netstat, resolvectl i networksetup oraz, w systemie Windows, za pomocą poleceń cmdlet NetTCPIP i DnsClient programu PowerShell. |

## Przykłady {#examples}

Skierowanie sieci przez bramę.

```nurt example
network.route_add "10.9.0.0/16" gateway: "203.0.113.5" privilege: elevated
```

Skierowanie sieci przez interfejs z metryką.

```nurt example
network.route_add "10.20.0.0/16" interface: "eth0" metric: 200 privilege: elevated
```

Ustawienie trasy domyślnej.

```nurt example
network.route_add "default" gateway: "203.0.113.1" privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | Wartość nie jest adresem, siecią ani nazwą interfejsu. | Popraw wartość. |
| `E_MISSING_PARAM` | Nie podano ani `gateway`, ani `interface`. | Podaj następny przeskok lub interfejs. |
| `E_PRIVILEGE` | Program systemu odmówił zmiany z braku uprawnień. | Powtórz wywołanie z `privilege: elevated`. |

## Powiązane {#related}

`network.route_remove` usuwa trasy, a `network.dns_set` ustawia serwery nazw.
