---
command: firewall.rules
group: administration
summary: Wyświetla reguły zapory wraz z ich działaniem, protokołem, portem, źródłem, kierunkiem i strefą.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# firewall.rules

## Składnia {#synopsis}

```text
firewall.rules [<port>] [name: value]...
```

## Opis {#description}

`firewall.rules` wyświetla reguły zapory jako rekordy z polami `backend`, `place` (tabela i łańcuch albo strefa), `managed` (regułę utworzył Nurt), `action` (`allow` lub `deny`), `protocol`, `port`, `direction`, `source`, `zone` oraz `rule` (reguła w postaci, w jakiej wypisuje ją zapora). Reguła o kształcie, którego Nurt nie modeluje, na przykład reguła bez portu, ma tylko `backend`, `place`, `managed` i `rule`. `port` i `protocol` zachowują tylko pasujące reguły.

Odczyt reguł większości zapór wymaga administratora: zapisz `privilege: elevated`.

Zaporę wybiera `backend`. Z wartością `auto` polecenie wybiera pierwszą zainstalowaną i działającą: w systemie Linux firewalld, następnie ufw, następnie nftables; w systemie macOS pf; w systemie Windows Zaporę Windows Defender.

| Zaplecze | System | Uwagi |
|---|---|---|
| `nftables` | Linux | Reguły znajdują się w łańcuchach `input` i `output` tabeli `table` (domyślnie `nurt`, rodzina `inet`); tabela i łańcuch są tworzone, gdy ich brakuje. Komentarz reguły zawiera jej klucz. `zone` nie jest dostępne. |
| `firewalld` | Linux | Reguła jest dodawana do konfiguracji uruchomieniowej i do trwałej. Zwykłe reguły zezwalające są portami; reguły z `source` i każda reguła blokująca są regułami rozszerzonymi (rich rules). `zone` domyślnie przyjmuje strefę domyślną firewalld; obsługiwany jest tylko ruch przychodzący. |
| `ufw` | Linux | Reguły są dodawane i usuwane za pomocą `ufw`; `zone` nie jest dostępne. |
| `pf` | macOS | Reguły znajdują się w kotwicy `nurt`, która zawiera wyłącznie reguły Nurt i jest ładowana w całości poleceniem `pfctl -a nurt -f -`. Kotwica zaczyna działać, gdy `pf.conf` odwołuje się do niej przez `anchor "nurt"`. `zone` nie jest dostępne. |
| `windows` | Windows | Reguły są tworzone za pomocą `New-NetFirewallRule` pod nazwą wyświetlaną `nurt:<key>`; `zone` wskazuje profil `domain`, `private` lub `public`. |

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `port` | `int \| text` | brak | Tylko reguły dla tego portu lub zakresu. |
| `protocol` | `enum(tcp\|udp)` | brak | Tylko reguły tego protokołu. |
| `backend` | `enum(auto\|nftables\|firewalld\|ufw\|pf\|windows)` | `auto` | Zapora do użycia; auto wybiera pierwszą zainstalowaną i działającą. |
| `table` | `text` | `"nurt"` | Tabela rodziny inet, która zawiera reguły nftables. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `any` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |
| External use | `system_program`: `nft`, `firewall-cmd`, `ufw`, `pfctl`, `powershell`; Reguły są odczytywane i zmieniane przez nft, firewall-cmd, ufw lub pfctl; w systemie Windows przez polecenia cmdlet NetSecurity programu PowerShell. |

## Przykłady {#examples}

Wyświetlenie każdej reguły zapory.

```nurt example
firewall.rules
```

Wyświetlenie reguł portu 22 w nftables.

```nurt example
firewall.rules 22 backend: nftables
```

Wyświetlenie tylko reguł utworzonych przez Nurt.

```nurt example
firewall.rules |> filter managed
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_PRIVILEGE` | Program systemowy odmówił zmiany z powodu braku uprawnień. | Powtórz wywołanie z `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Nie znaleziono programu systemowego, który to potrafi. | Zainstaluj program albo wskaż `backend`. |
| `E_OS_ONLY` | Zaplecze nie istnieje w tym systemie. | Użyj `backend: auto`. |

## Powiązane {#related}

`firewall.allow`, `firewall.deny` i `firewall.remove` zmieniają reguły.
