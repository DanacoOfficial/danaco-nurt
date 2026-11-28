---
command: network.interfaces
group: network
summary: Wyświetla interfejsy sieciowe z adresami, adresem MAC, stanem i MTU.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# network.interfaces

## Składnia {#synopsis}

```text
network.interfaces
```

## Opis {#description}

`network.interfaces` zwraca jeden rekord `network_interface` dla każdego
interfejsu sieciowego hosta, uporządkowany według nazwy. Adresy aliasu, takiego
jak `eth0:1`, należą do interfejsu `eth0`. Brama domyślna interfejsu jest
pobierana z tablicy tras IPv4, a w przeciwnym razie z tablicy tras IPv6.

Pola są odczytywane z następujących interfejsów systemu operacyjnego.

| System | Adresy | Stan, MAC, MTU, rodzaj | Brama |
|---|---|---|---|
| Linux | `getifaddrs` | `/sys/class/net/<name>/{operstate,flags,address,mtu,type}`; `wireless` lub `phy80211` daje `wireless`, brak `device` daje `virtual` | `/proc/net/route`, `/proc/net/ipv6_route` |
| macOS | `getifaddrs` | wpisy warstwy łącza z `getifaddrs` i dane ich interfejsu; typ łącza i przedrostek nazwy wyznaczają rodzaj | tablica tras odczytywana przez `sysctl` (`NET_RT_FLAGS`) |
| Windows | `GetAdaptersAddresses` | wpis karty; typ interfejsu wyznacza rodzaj | bramy karty, najpierw IPv4 |

W systemie Linux `operstate` o wartości `unknown`, zgłaszany przez interfejs
pętli zwrotnej i niektóre interfejsy wirtualne, jest rozstrzygany flagą
`IFF_UP`.

## Parametry {#parameters}

Polecenie nie ma parametrów.

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `network_interface` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie wszystkich interfejsów:

```nurt example
network.interfaces
```

```text
network_interface  lo    127.0.0.1/8, ::1/128         up  loopback
network_interface  eth0  192.0.2.10/24, 2001:db8::10/64  up  ethernet  gateway 192.0.2.1
```

Wyświetlenie interfejsów, które są włączone i mają bramę domyślną:

```nurt example
network.interfaces |> filter up and gateway != null
```

Wyświetlenie adresów sprzętowych interfejsów bezprzewodowych:

```nurt example
network.interfaces |> filter kind == "wireless" |> select name mac
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_IO` | System odmawia listy interfejsów albo w systemie Linux nie można odczytać `/sys/class/net`. | Sprawdź stan systemu i uruchom program ponownie. |
| `E_OS_ONLY` | System nie jest systemem Linux, macOS ani Windows. | Uruchom program w obsługiwanym systemie. |

## Powiązane {#related}

`network.ping`, `network.resolve`, `ports`.
