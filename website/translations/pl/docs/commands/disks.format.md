---
command: disks.format
group: administration
summary: Tworzy system plików na urządzeniu.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# disks.format

## Składnia {#synopsis}

```text
disks.format <device> fs: <enum(ext4|xfs|btrfs|vfat|ntfs|exfat)> [name: value]...
```

## Opis {#description}

`disks.format` tworzy system plików za pomocą `mkfs.<type>` w systemie Linux, `diskutil eraseVolume` w systemie macOS i `Format-Volume` w systemie Windows. Zmiana jest nieodwracalna. Podgląd i raport wymieniają urządzenie, jego partycje, systemy plików, punkty montowania oraz rozmiar danych, które zostaną utracone; urządzenie z zamontowaną partycją jest odrzucane z kodem `E_IN_USE`. `fs` przyjmuje wartości `ext4`, `xfs`, `btrfs`, `vfat`, `ntfs` lub `exfat` (macOS akceptuje `vfat` i `exfat`).

Uprawnienia: polecenie przyjmuje `privilege`; zapisz `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który kończy się niepowodzeniem z powodu braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je jednokrotnie z podwyższonymi uprawnieniami. Wpis z podwyższonymi uprawnieniami jest cofany przez ten sam proces z podwyższonymi uprawnieniami.

Formatowanie jest nieodwracalne: jedynym zabezpieczeniem jest podgląd.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `device` | `text` | wymagany | Partycja lub dysk, na przykład /dev/sdb1, albo litera dysku, na przykład D: w systemie Windows. |
| `fs` | `enum(ext4\|xfs\|btrfs\|vfat\|ntfs\|exfat)` | wymagany | System plików. |
| `label` | `text` | brak | Etykieta systemu plików. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `external` |
| Odwracalność | `irreversible` |
| Implementacja | `system_program` |
| Warunki odwracalności | Nieodwracalne; podgląd wymienia urządzenie, jego partycje, systemy plików, punkty montowania oraz rozmiar danych, które zostaną utracone. |
| External use | `system_program`: `mkfs`, `lsblk`, `diskutil`, `powershell`; Systemy plików są tworzone za pomocą mkfs w systemie Linux, diskutil w systemie macOS i Format-Volume w systemie Windows. |

## Przykłady {#examples}

Uruchomienie formatowania z podwyższonymi uprawnieniami; dodanie parametru wywołania `preview: true` najpierw pokazuje skutek.

```nurt example
disks.format "/dev/sdb1" fs: ext4 label: "data" privilege: elevated
```

Formatowanie partycji.

```nurt example
disks.format "/dev/sdb1" fs: xfs label: "backup" privilege: elevated
```

Formatowanie dysku w systemie Windows.

```nurt example
disks.format "D:" fs: ntfs label: "Data" privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_IN_USE` | Partycja urządzenia jest zamontowana. | Najpierw ją odmontuj. |
| `E_INVALID_VALUE` | Urządzenie nie jest urządzeniem blokowym albo system plików jest nieznany. | Sprawdź urządzenie poleceniem `disks`. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wskazany w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`disks`, `disks.partition`, `unmount`.
