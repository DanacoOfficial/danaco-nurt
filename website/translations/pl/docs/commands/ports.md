---
command: ports
group: system
summary: Wyświetla porty sieciowe i procesy, które je trzymają.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# ports

## Składnia {#synopsis}

```text
ports [name: value]...
```

## Opis {#description}

`ports` wyświetla gniazda TCP i UDP hosta jako rekordy `port`, uporządkowane
według protokołu, portu lokalnego, adresu lokalnego i procesu. Z `state:
listen` wyjście zawiera nasłuchujące gniazda TCP i gniazda UDP bez adresu
zdalnego; z `state: all` zawiera każde gniazdo, w tym połączenia nawiązane i
zamykane.

`local` jest adresem lokalnym i portem w postaci `address:port`, z adresem IPv6
w nawiasach kwadratowych, na przykład `[::1]:8080`. `remote`, `remote_address`
i `remote_port` są ustawiane tylko dla gniazd z drugą stroną. Gniazda UDP mają
stan `none`. `pid` i `process` wskazują proces, który trzyma gniazdo.

| System | Tablice gniazd | Proces trzymający |
|---|---|---|
| Linux | `/proc/net/tcp`, `tcp6`, `udp`, `udp6` | deskryptor w `/proc/<pid>/fd` dowiązany do i-węzła gniazda |
| macOS | `proc_pidfdinfo(PROC_PIDFDSOCKETINFO)` dla każdego procesu | wymieniony proces |
| Windows | `GetExtendedTcpTable`, `GetExtendedUdpTable` | identyfikator procesu z wiersza tablicy |

W systemie Linux proces trzymający gniazdo innego konta jest znany tylko
koncie uprzywilejowanemu; takie gniazda mają pusty `pid`. W systemie macOS
gniazda procesów innych kont są wyświetlane tylko dla konta uprzywilejowanego.
W obu przypadkach raport zawiera jedną uwagę `source_unavailable` z liczbą
dotkniętych gniazd lub procesów. Stany TCP są odwzorowywane na `listen`,
`established`, `syn_sent`, `syn_received`, `fin_wait`, `close_wait`,
`closing`, `last_ack`, `time_wait` i `closed`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `state` | `enum(listen\|all)` | `listen` | Tylko porty nasłuchujące albo wszystkie gniazda. |
| `protocol` | `enum(tcp\|udp\|any)` | `any` | Protokół transportowy. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `port` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie nasłuchujących portów TCP:

```nurt example
ports state: listen protocol: tcp
```

Wyświetlenie wszystkich gniazd UDP:

```nurt example
ports state: all protocol: udp
```

Wyświetlenie wszystkich gniazd wraz z ich stanami i procesami:

```nurt example
ports state: all
```

Odpowiedź, która pomija rekordy, kończy się wskazówką „more: next” i zawiera
pole `continuation`; polecenie `next` zwraca kolejną część bez ponownego
uruchamiania polecenia.

```text
matched 85  shown 50  more: next
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | `state` lub `protocol` ma wartość spoza wyliczenia. | Użyj jednej z wymienionych wartości. |
| `E_PRIVILEGE` | Konto nie może odczytać tablic gniazd. | Uruchom z uprawnieniem wskazanym w `required`. |
| `E_IO` | Nie można odczytać tablicy gniazd. | Sprawdź stan systemu i uruchom program ponownie. |

## Powiązane {#related}

`processes`, `network.interfaces`, `open_files`.
