---
command: disks
group: system
summary: Wyświetla zamontowane woluminy z pojemnością i wolnym miejscem.
since: 0.1.0
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# disks

## Składnia {#synopsis}

```text
disks [name: value]...
```

## Opis {#description}

`disks` wyświetla zamontowane woluminy hosta jako rekordy `disk`, uporządkowane
według `mount`. Pseudosystemy plików jądra, takie jak `proc`, `sysfs`,
`cgroup2`, `devpts` i `autofs`, nie są woluminami i są pomijane. Gdy w tym
samym punkcie zamontowano kilka systemów plików, wyświetlany jest tylko
widoczny.

`type` wybiera woluminy według rodzaju. Rodzaj woluminu wynika z reguł
przestrzeni dziennika:

| `type` | Woluminy |
|---|---|
| `network` | `nfs`, `nfs4`, `cifs`, `smb3`, `smbfs`, `sshfs`, `fuse.sshfs`, `9p`, `afs`, `afpfs`, `webdav`; w systemie macOS każdy wolumin bez `MNT_LOCAL`; w systemie Windows `DRIVE_REMOTE` |
| `ram` | `tmpfs`, `ramfs`, `devtmpfs`; w systemie Windows `DRIVE_RAMDISK` |
| `removable` | nośniki wymienne oraz, w systemie macOS, urządzenia zewnętrzne |
| `fixed` | każdy inny wolumin lokalny, w tym system plików `overlay` kontenerów |
| `other` | systemy plików w przestrzeni użytkownika inne niż wymienione sieciowe systemy plików |

Woluminy `ram` i `other` pojawiają się tylko z `type: all`.

`total` to pojemność, `free` to miejsce dostępne dla konta, a `used` to
pojemność pomniejszona o wszystkie wolne bloki, więc `used` i `free` nie sumują
się do `total` w systemach plików, które rezerwują bloki. `inodes_total` oraz
`inodes_free` są ustawiane w systemach Linux i macOS dla systemów plików, które
je zgłaszają. `options` zawiera opcje montowania w systemie Linux.

| System | Woluminy | Miejsce | Nośniki wymienne | Etykieta |
|---|---|---|---|---|
| Linux | `/proc/self/mountinfo` | `statvfs` | atrybut `removable` urządzenia blokowego | `/dev/disk/by-label` |
| macOS | `getmntinfo` | `statfs` | klucze zasobów woluminu | nazwa woluminu |
| Windows | litery dysków | `GetDiskFreeSpaceExW` | `GetDriveTypeW` | `GetVolumeInformationW` |

W systemie Windows `mount` jest katalogiem głównym dysku w postaci kanonicznej,
na przykład `C:/`. Wolumin, którego miejsca nie można odczytać, na przykład
nieosiągalny udział sieciowy, jest pomijany; raport zawiera jedną uwagę
`source_unavailable` z liczbą takich woluminów.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `type` | `enum(all\|fixed\|removable\|network)` | `all` | Rodzaj woluminu. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `disk` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `source` |
| Skutek | `read` |
| Odwracalność | `read` |
| Implementacja | `builtin` |

## Przykłady {#examples}

Wyświetlenie woluminów sieciowych:

```nurt example
disks type: network
```

Wyświetlenie nośników wymiennych:

```nurt example
disks type: removable
```

Wyświetlenie wszystkich zamontowanych woluminów:

```nurt example
disks
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_INVALID_VALUE` | `type` ma wartość spoza wyliczenia. | Użyj jednej z wymienionych wartości. |
| `E_IO` | Nie można odczytać tabeli montowania. | Sprawdź, czy `/proc` jest zamontowany. |

## Powiązane {#related}

`journal.space`, `host`, `list`.
