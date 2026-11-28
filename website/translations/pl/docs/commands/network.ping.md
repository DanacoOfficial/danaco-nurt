---
command: network.ping
group: network
summary: Wysyła żądania echo ICMP do hosta i zgłasza czasy odpowiedzi.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# network.ping

## Składnia {#synopsis}

```text
network.ping <host> [name: value]...
```

## Opis {#description}

`network.ping` wysyła `count` żądań echo ICMP do `host`, w odstępach jednej
sekundy, i zwraca jeden rekord `ping_result` z liczbą odpowiedzi oraz czasami
odpowiedzi. Nazwa hosta jest rozwiązywana przez resolver systemu
operacyjnego, który odczytuje także plik hosts; używany jest pierwszy adres.
Adres jest używany tak, jak podano, IPv4 lub IPv6.

Żądanie bez odpowiedzi w czasie `timeout` jest liczone jako utracone.
Odpowiedzi są dopasowywane do żądań według numeru sekwencyjnego oraz losowego
znacznika w danych, więc odpowiedzi do innych programów są ignorowane.

| System | Mechanizm |
|---|---|
| Linux | gniazdo datagramowe ICMP (`SOCK_DGRAM` z `IPPROTO_ICMP` lub `IPPROTO_ICMPV6`) |
| macOS | gniazdo datagramowe ICMP |
| Windows | `IcmpSendEcho` dla IPv4, `Icmp6SendEcho2` dla IPv6 |

Gniazda surowe nie są używane.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `host` | `text` | wymagany | Nazwa hosta lub adres. |
| `count` | `int` | `4` | Liczba żądań. |
| `timeout` | `duration` | `2s` | Czas oczekiwania na każdą odpowiedź. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `ping_result` |

Zdarzenia postępu fazy `echo` zgłaszają wysłane żądania.

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

W systemie Linux grupa konta musi należeć do `net.ipv4.ping_group_range`; w
systemach macOS i Windows nie jest wymagane żadne uprawnienie. Gdy Linux
odmawia gniazda, błąd podaje bieżącą wartość `net.ipv4.ping_group_range` i
proponuje poprawkę, która dopuszcza każdą grupę.

## Przykłady {#examples}

Wysłanie czterech żądań do hosta:

```nurt example
network.ping "example.org"
```

```text
ping_result  example.org  93.184.215.14  sent 4  received 4  loss 0
```

Wysłanie dziesięciu żądań do adresu IPv6 z oczekiwaniem do jednej sekundy na każde:

```nurt example
network.ping "2001:db8::1" count: 10 timeout: 1s
```

Zgłoszenie tylko utraty pakietów i średniego czasu odpowiedzi bramy:

```nurt example
network.ping "192.168.1.1" count: 20 |> select loss rtt_avg
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | `count` jest mniejsze niż 1, `timeout` wynosi zero albo `host` jest pusty. | Podaj dodatnią liczbę i limit czasu oraz host. |
| `E_NETWORK` | Nazwy hosta nie można rozwiązać (`problem` `dns`) albo nie można wysłać żądania (`problem` `icmp`). | Sprawdź nazwę i sieć. |
| `E_PRIVILEGE` | System odmawia kontu gniazda ICMP. | Uruchom program w `fix`, który rozszerza `net.ipv4.ping_group_range`. |
| `E_TIMEOUT` | Program osiągnął swój limit czasu. | Zmniejsz `count` albo zwiększ limit programu. |
| `E_CANCELLED` | Wykonanie zostało anulowane. | Uruchom polecenie ponownie. |

## Powiązane {#related}

`network.resolve`, `network.interfaces`, `http.request`.
