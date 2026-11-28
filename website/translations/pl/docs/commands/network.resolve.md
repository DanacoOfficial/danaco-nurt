---
command: network.resolve
group: network
summary: Rozwiązuje rekordy DNS nazwy.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# network.resolve

## Składnia {#synopsis}

```text
network.resolve <name> [name: value]...
```

## Opis {#description}

`network.resolve` wysyła zapytanie DNS o `name` i zwraca rekordy odpowiedzi
jako rekordy `dns_record`, po jednym na rekord. Nazwa jest odpytywana tak, jak
podano, bez domen wyszukiwania. Odpowiedź bez rekordów danego typu nie daje
rekordów; nazwa, która nie istnieje, kończy się błędem `E_NOT_FOUND`.

Bez `server` odpytywane są po kolei serwery z konfiguracji systemu, aż któryś
odpowie: wiersze `nameserver` z `/etc/resolv.conf` w systemach Linux i macOS
(co najwyżej trzy, z `options timeout:` i `attempts`) oraz serwery DNS kart
sieciowych w systemie Windows. Z `server` odpytywany jest tylko ten serwer.

Każde zapytanie jest jednym datagramem UDP. Obcięta odpowiedź jest powtarzana
przez TCP. `type: any` wysyła jedno zapytanie na typ, w kolejności A, AAAA, MX,
TXT, NS, CNAME, i scala rekordy bez duplikatów; nie wysyła zapytania typu
`ANY`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text` | wymagany | Nazwa domeny. |
| `type` | `enum(a\|aaaa\|mx\|txt\|ns\|cname\|any)` | `a` | Typ rekordu. |
| `server` | `text` | brak | Serwer DNS do odpytania zamiast resolvera systemowego. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `dns_record` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Rozwiązanie adresów IPv4 nazwy:

```nurt example
network.resolve "example.org"
```

```text
dns_record  example.org  A  93.184.215.14  ttl 3600s
```

Zapytanie wskazanego serwera o serwery poczty domeny:

```nurt example
network.resolve "example.org" type: mx server: "192.0.2.53"
```

Wyświetlenie rekordów każdego obsługiwanego typu, uporządkowanych według typu:

```nurt example
network.resolve "example.org" type: any |> sort by: type
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | `type` nie jest jedną z wymienionych wartości, `name` nie jest poprawną nazwą domeny albo `server` nie jest adresem i nie można go rozwiązać. | Popraw wartość zgodnie z opisem w komunikacie. |
| `E_NOT_FOUND` | Serwer odpowiada, że nazwa nie istnieje. | Sprawdź pisownię nazwy. |
| `E_NETWORK` | Żaden serwer nie odpowiada albo każdy serwer zgłasza niepowodzenie (`problem` `dns`, serwer w `server`). | Sprawdź sieć albo podaj inny serwer w `server`. |
| `E_IO` | Konfiguracja systemu istnieje, ale nie można jej odczytać. | Sprawdź uprawnienia do `/etc/resolv.conf`. |
| `E_TIMEOUT` | Program osiągnął swój limit czasu. | Zwiększ limit programu. |
| `E_CANCELLED` | Wykonanie zostało anulowane. | Uruchom polecenie ponownie. |

## Powiązane {#related}

`network.ping`, `network.interfaces`, `http.request`.
