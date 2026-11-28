---
command: firewall.remove
group: administration
summary: Usuwa reguły zapory pasujące do portu, protokołu, źródła, kierunku i strefy.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# firewall.remove

## Składnia {#synopsis}

```text
firewall.remove <port> [name: value]...
```

## Opis {#description}

`firewall.remove` usuwa reguły pasujące do `port`, `protocol`, `source`, `direction` i `zone`. `action` ogranicza usuwanie do reguł zezwalających lub blokujących; z wartością `any` usuwane są obie. Wywołanie, które nie pasuje do żadnej reguły, kończy się błędem `E_NOT_FOUND` i niczego nie zmienia. Każda usunięta reguła jest jedną zmianą celu `firewall:<rule>` z `present` na `absent`; `undo` dodaje reguły ponownie za pomocą `firewall.allow` lub `firewall.deny`. Reguły są opisywane przez `port`, `protocol`, `source`, `direction` i `zone`. `source` to zdalny adres ruchu: nadawca ruchu przychodzącego i odbiorca ruchu wychodzącego. Port to liczba od 1 do 65535 albo zakres `first-last`.

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
| `port` | `int \| text` | wymagany | Port lub zakres portów, na przykład 8000-8100. |
| `protocol` | `enum(tcp\|udp)` | `tcp` | Protokół transportowy. |
| `source` | `text` | brak | Zdalny adres lub sieć w postaci CIDR: nadawca ruchu przychodzącego i odbiorca ruchu wychodzącego; dowolny, gdy pominięty. |
| `direction` | `enum(inbound\|outbound)` | `inbound` | Kierunek ruchu. |
| `zone` | `text` | brak | Strefa firewalld lub profil systemu Windows (domain, private, public); domyślna, gdy pominięta. |
| `backend` | `enum(auto\|nftables\|firewalld\|ufw\|pf\|windows)` | `auto` | Zapora do użycia; auto wybiera pierwszą zainstalowaną i działającą. |
| `table` | `text` | `"nurt"` | Tabela rodziny inet, która zawiera reguły nftables. |
| `action` | `enum(any\|allow\|deny)` | `any` | Usuwa tylko reguły z tym działaniem; obie, gdy any. |

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
| Warunki odwracalności | `undo` dodaje usunięte reguły. |
| External use | `system_program`: `nft`, `firewall-cmd`, `ufw`, `pfctl`, `powershell`; Reguły są odczytywane i zmieniane przez nft, firewall-cmd, ufw lub pfctl; w systemie Windows przez polecenia cmdlet NetSecurity programu PowerShell. |

## Przykłady {#examples}

Usunięcie reguły dla portu 22.

```nurt example
firewall.remove 22 privilege: elevated
```

Usunięcie tylko reguły blokującej port UDP.

```nurt example
firewall.remove 5353 protocol: udp action: deny privilege: elevated
```

Usunięcie reguły z tabeli nftables o podanej nazwie.

```nurt example
firewall.remove 8080 backend: nftables table: "nurt" privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | Żadna reguła nie pasuje. | Wyświetl reguły poleceniem `firewall.rules`. |
| `E_INVALID_VALUE` | `port`, `source` lub `zone` jest nieprawidłowe. | Zapisz port od 1 do 65535, adres w postaci CIDR albo zwykłą nazwę. |
| `E_PRIVILEGE` | Program systemowy odmówił zmiany z powodu braku uprawnień. | Powtórz wywołanie z `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Nie znaleziono programu systemowego, który to potrafi. | Zainstaluj program albo wskaż `backend`. |

## Powiązane {#related}

`firewall.rules`, `firewall.allow`, `firewall.deny`.
