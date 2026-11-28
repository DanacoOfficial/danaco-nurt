---
command: sysctl.set
group: administration
summary: Ustawia parametr jądra.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# sysctl.set

## Składnia {#synopsis}

```text
sysctl.set <key> <value> [name: value]...
```

## Opis {#description}

`sysctl.set` ustawia parametr jądra za pomocą `sysctl -w` w systemach Linux i macOS. Bieżąca wartość jest odczytywana za pomocą `sysctl -n` i zapisywana; parametr, który już ma tę wartość, jest `unchanged`. Przy `persistent: true` w systemie Linux wiersz `key = value` jest zapisywany do `/etc/sysctl.d/90-nurt.conf` przez dziennik.

Uprawnienia: polecenie przyjmuje `privilege`; zapisz `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który zawodzi z braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je jednokrotnie z podwyższonymi uprawnieniami. Wpis z podwyższonymi uprawnieniami jest cofany przez ten sam proces z podwyższonymi uprawnieniami.

`undo` uruchamia `sysctl.set` z zapisaną wartością; dziennik przywraca plik.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `key` | `text` | wymagany | Nazwa parametru, na przykład net.ipv4.ip_forward. |
| `value` | `text \| int` | wymagany | Nowa wartość. |
| `persistent` | `bool` | `false` | Zapisuje wartość także do /etc/sysctl.d/90-nurt.conf (Linux); plik jest rejestrowany w dzienniku. |

## Wejście i wyjście {#input-and-output}

| Forma | Wejście | Wyjście |
|---|---|---|
| default | `none` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `external` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `system_program` |
| Warunki odwracalności | Cofnięcie ustawia zapisaną wartość; plik jest przywracany przez dziennik. |
| Systemy operacyjne | `linux`, `macos` |
| External use | `system_program`: `sysctl`; parametry są odczytywane i ustawiane za pomocą sysctl. |

## Przykłady {#examples}

Włączenie przekazywania pakietów.

```nurt example
sysctl.set "net.ipv4.ip_forward" 1 privilege: elevated
```

Ustawienie wartości z zachowaniem jej po ponownym uruchomieniu.

```nurt example
sysctl.set "vm.swappiness" 10 persistent: true privilege: elevated
```

Wykonanie zmiany z podwyższonymi uprawnieniami; dodanie parametru wywołania `preview: true` najpierw pokazuje skutek.

```nurt example
sysctl.set "vm.swappiness" 10 privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | Jądro nie ma takiego parametru. | Sprawdź nazwę za pomocą `sysctl -a`. |
| `E_UNSUPPORTED` | `persistent` poza systemem Linux. | Pomiń `persistent`. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wskazany w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`modules.load`, `modules.unload`.
