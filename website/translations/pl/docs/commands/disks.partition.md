---
command: disks.partition
group: administration
summary: Tworzy, usuwa lub zmienia rozmiar partycji.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# disks.partition

## Składnia {#synopsis}

```text
disks.partition <device> action: <enum(create|delete|resize)> [name: value]...
```

## Opis {#description}

`disks.partition` tworzy, usuwa lub zmienia rozmiar partycji za pomocą `parted` w systemie Linux, `diskutil` w systemie macOS i poleceń cmdlet magazynu w systemie Windows. Zmiana jest nieodwracalna. Podgląd i raport wymieniają urządzenie, jego partycje wraz z systemami plików, etykietami i punktami montowania oraz rozmiar danych, które zmiana zniszczy; zamontowana partycja jest odrzucana z kodem `E_IN_USE`. `device` to dysk (`/dev/sdb` albo numer dysku w systemie Windows); `number` wybiera partycję do usunięcia lub zmiany rozmiaru; nowa partycja zaczyna się w `start` (domyślnie 1 MiB) i zajmuje `size` albo resztę dysku.

Uprawnienia: polecenie przyjmuje `privilege`; zapisz `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który kończy się niepowodzeniem z powodu braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je jednokrotnie z podwyższonymi uprawnieniami. Wpis z podwyższonymi uprawnieniami jest cofany przez ten sam proces z podwyższonymi uprawnieniami.

Zmiana partycji jest nieodwracalna: jedynym zabezpieczeniem jest podgląd.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `device` | `text` | wymagany | Urządzenie dysku, na przykład /dev/sdb, albo numer dysku w systemie Windows. |
| `action` | `enum(create\|delete\|resize)` | wymagany | Zmiana do wykonania. |
| `number` | `int` | brak | Numer partycji do usunięcia lub zmiany rozmiaru. |
| `start` | `size` | brak | Początek nowej partycji; 1 MiB, gdy pominięty. |
| `size` | `size` | brak | Rozmiar nowej partycji lub partycji o zmienianym rozmiarze; reszta dysku, gdy pominięty przy create. |
| `fs_type` | `text` | brak | Wskazówka typu systemu plików nowej partycji. |

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
| External use | `system_program`: `parted`, `lsblk`, `diskutil`, `powershell`; Partycje są zmieniane za pomocą parted w systemie Linux, diskutil w systemie macOS i poleceń cmdlet magazynu w systemie Windows. |

## Przykłady {#examples}

Uruchomienie usunięcia partycji z podwyższonymi uprawnieniami; dodanie parametru wywołania `preview: true` najpierw pokazuje skutek.

```nurt example
disks.partition "/dev/sdb" action: delete number: 2 privilege: elevated
```

Utworzenie partycji o rozmiarze 10 GiB.

```nurt example
disks.partition "/dev/sdb" action: create size: 10gib privilege: elevated
```

Zmiana rozmiaru partycji.

```nurt example
disks.partition "/dev/sdb" action: resize number: 1 size: 20gib privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_IN_USE` | Partycja jest zamontowana. | Najpierw ją odmontuj. |
| `E_NOT_FOUND` | Partycja nie istnieje. | Sprawdź numer poleceniem `disks`. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wskazany w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`disks`, `disks.format`, `unmount`.
