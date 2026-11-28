---
command: unmount
group: administration
summary: Odmontowuje punkt montowania.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# unmount

## Składnia {#synopsis}

```text
unmount <target> [name: value]...
```

## Opis {#description}

`unmount` odmontowuje punkt montowania za pomocą `umount` w systemach Linux i macOS oraz `mountvol /D` w systemie Windows. Urządzenie, typ i opcje są odczytywane najpierw. Przy `persistent: true` w systemie Linux wiersz punktu montowania jest usuwany z `/etc/fstab` przez dziennik.

Uprawnienia: polecenie przyjmuje `privilege`; zapisz `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który zawodzi z braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je jednokrotnie z podwyższonymi uprawnieniami. Wpis z podwyższonymi uprawnieniami jest cofany przez ten sam proces z podwyższonymi uprawnieniami.

`undo` uruchamia `mount` z zapisanym urządzeniem, typem i opcjami.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `target` | `text` | wymagany | Punkt montowania. |
| `persistent` | `bool` | `false` | Usuwa montowanie także z /etc/fstab (Linux); plik jest rejestrowany w dzienniku. |

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
| Warunki odwracalności | Cofnięcie montuje zapisane urządzenie z jego typem i opcjami. |
| External use | `system_program`: `umount`, `mountvol`; odmontowanie używa umount w systemach Linux i macOS oraz mountvol w systemie Windows. |

## Przykłady {#examples}

Odmontowanie katalogu.

```nurt example
unmount "/srv/data" privilege: elevated
```

Odmontowanie i usunięcie wiersza z `/etc/fstab`.

```nurt example
unmount "/srv/data" persistent: true privilege: elevated
```

Wykonanie odmontowania z podwyższonymi uprawnieniami; dodanie parametru wywołania `preview: true` najpierw pokazuje skutek.

```nurt example
unmount "/srv/data" privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | W punkcie montowania nic nie jest zamontowane. | Sprawdź punkt montowania za pomocą `disks`. |
| `E_IN_USE` | Proces używa montowania. | Zatrzymaj procesy wymienione przez `open_files`. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wskazany w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`mount`, `disks`.
