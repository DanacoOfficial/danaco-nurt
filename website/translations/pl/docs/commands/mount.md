---
command: mount
group: administration
summary: Montuje urządzenie lub wolumin.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# mount

## Składnia {#synopsis}

```text
mount <device> <target> [name: value]...
```

## Opis {#description}

`mount` montuje urządzenie lub wolumin za pomocą `mount` w systemach Linux i macOS oraz `mountvol` w systemie Windows (litery dysków i montowanie w folderach). Punkt montowania jest odczytywany najpierw: to samo urządzenie już zamontowane w tym miejscu daje wynik `unchanged`, a inne urządzenie daje `E_IN_USE`. Z `persistent: true` w systemie Linux wiersz pliku `/etc/fstab` dla punktu montowania jest zapisywany lub zastępowany; edycja jest zmianą pliku w dzienniku. macOS nie obsługuje `persistent`.

Uprawnienia: polecenie przyjmuje `privilege`; wpisz `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który kończy się niepowodzeniem z braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je po podniesieniu uprawnień. Wpis z podniesionymi uprawnieniami jest cofany przez ten sam proces z podniesionymi uprawnieniami.

Cofnięcie (`undo`) uruchamia `unmount`; dziennik przywraca `/etc/fstab`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `device` | `text` | wymagany | Urządzenie, wolumin lub źródło montowania. |
| `target` | `text` | wymagany | Punkt montowania: katalog albo litera dysku lub folder w systemie Windows. |
| `type` | `text` | brak | Typ systemu plików; wykrywany przez system, gdy pominięty. |
| `options` | `list<text>` | brak | Opcje montowania. |
| `persistent` | `bool` | `false` | Zapisuje montowanie także w /etc/fstab (Linux); plik jest rejestrowany w dzienniku. |

## Wejście i wyjście {#input-and-output}

| Postać | Wejście | Wyjście |
|---|---|---|
| domyślna | `none` | `effect` |

## Skutek {#effect}

| Właściwość | Wartość |
|---|---|
| Kategoria | `change` |
| Skutek | `external` |
| Odwracalność | `reversible_if_preserved` |
| Implementacja | `system_program` |
| Warunki odwracalności | Cofnięcie odmontowuje cel; edycja /etc/fstab jest przywracana przez dziennik. |
| External use | `system_program`: `mount`, `mountvol`; Montowanie używa mount w systemach Linux i macOS oraz mountvol w systemie Windows. |

## Przykłady {#examples}

Zamontowanie urządzenia.

```nurt example
mount "/dev/sdb1" "/srv/data" type: "ext4" privilege: elevated
```

Zamontowanie w trybie tylko do odczytu z zachowaniem montowania po ponownym uruchomieniu.

```nurt example
mount "/dev/sdb1" "/srv/data" type: "ext4" options: ["ro", "noatime"] persistent: true privilege: elevated
```

Zamontowanie nośnika USB dla jednego konta.

```nurt example
mount "/dev/sdc1" "/mnt/usb" type: "vfat" options: ["uid=1000"] privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_IN_USE` | Inne urządzenie jest zamontowane w punkcie montowania. | Najpierw je odmontuj. |
| `E_UNSUPPORTED` | `persistent` w systemie macOS. | Pomiń `persistent`. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wymieniony w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`unmount`, `disks`, `disks.format`.
