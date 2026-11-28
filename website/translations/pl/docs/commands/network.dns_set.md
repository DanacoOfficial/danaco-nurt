---
command: network.dns_set
group: administration
summary: Ustawia serwery DNS i domeny wyszukiwania interfejsu; pusta lista serwerów przywraca serwery automatyczne.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# network.dns_set

## Składnia {#synopsis}

```text
network.dns_set <servers> [name: value]...
```

## Opis {#description}

`network.dns_set` ustawia serwery nazw oraz, z `search`, domeny wyszukiwania interfejsu. Pusta lista `servers` przywraca interfejsowi serwery automatyczne. Pominięte `search` zachowuje domeny wyszukiwania.

| System | Mechanizm |
|---|---|
| Linux z systemd-resolved | `resolvectl dns`, `resolvectl domain`, `resolvectl revert`; `interface` domyślnie wskazuje interfejs trasy domyślnej |
| Linux w pozostałych przypadkach | plik `/etc/resolv.conf`, przepisywany przez dziennik, aby `undo` przywracało plik; pozostałe wiersze pliku są zachowywane; dowiązanie jest odrzucane błędem `E_UNSUPPORTED` |
| macOS | `networksetup -setdnsservers` i `-setsearchdomains`; `interface` jest nazwą usługi sieciowej |
| Windows | `Set-DnsClientServerAddress` dla aliasu interfejsu, `Set-DnsClientGlobalSetting` dla domen wyszukiwania |

Gdy serwery i domeny już mają te wartości, nic nie jest zmieniane. Cofnięcie (`undo`) ustawia poprzednie serwery i domeny za pomocą `network.dns_set`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `servers` | `list<text>` | wymagany | Adresy IP serwerów nazw, w kolejności; pusta lista oznacza automatyczne. |
| `search` | `list<text>` | brak | Domeny wyszukiwania; bez zmian, gdy pominięte. |
| `interface` | `text` | brak | Interfejs lub usługa sieciowa; w systemie Linux interfejs trasy domyślnej, gdy pominięty. |

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
| Warunki odwracalności | `undo` ustawia poprzednie serwery i domeny; zmiany /etc/resolv.conf są przywracane przez dziennik. |
| External use | `system_program`: `ip`, `route`, `netstat`, `resolvectl`, `networksetup`, `powershell`; Trasy i DNS są odczytywane i zmieniane za pomocą ip, route, netstat, resolvectl i networksetup oraz, w systemie Windows, za pomocą poleceń cmdlet NetTCPIP i DnsClient programu PowerShell. |

## Przykłady {#examples}

Użycie dwóch publicznych serwerów nazw na interfejsie.

```nurt example
network.dns_set ["1.1.1.1", "8.8.8.8"] interface: "eth0" privilege: elevated
```

Ustawienie serwerów i domen wyszukiwania.

```nurt example
network.dns_set ["198.51.100.2"] search: ["corp.example.com"] interface: "eth0" privilege: elevated
```

Przywrócenie interfejsowi serwerów automatycznych.

```nurt example
network.dns_set [] interface: "eth0" privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | Serwer nie jest adresem IP albo domena nie jest nazwą domeny. | Popraw wartość. |
| `E_MISSING_PARAM` | `interface` jest potrzebny i nie został podany. | Podaj nazwę interfejsu. |
| `E_UNSUPPORTED` | `/etc/resolv.conf` jest dowiązaniem zarządzanym przez inną usługę. | Ustaw DNS w tej usłudze albo włącz systemd-resolved. |
| `E_PRIVILEGE` | Program systemu odmówił zmiany z braku uprawnień. | Powtórz wywołanie z `privilege: elevated`. |

## Powiązane {#related}

`network.route_add`, `network.route_remove`.
