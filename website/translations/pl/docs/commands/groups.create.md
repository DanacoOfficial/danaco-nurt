---
command: groups.create
group: administration
summary: Tworzy grupę.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# groups.create

## Składnia {#synopsis}

```text
groups.create <name> [name: value]...
```

## Opis {#description}

`groups.create` tworzy grupę poleceniem `groupadd` w systemie Linux, `dseditgroup` w systemie macOS i `net localgroup` w systemie Windows. `gid` ustala identyfikator, a `system` tworzy grupę systemową w systemie Linux.

Uprawnienia: polecenie akceptuje `privilege`; zapisz `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który kończy się niepowodzeniem z powodu braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je jednokrotnie z podwyższonymi uprawnieniami. Wpis wykonany z podwyższonymi uprawnieniami jest cofany przez ten sam proces z podwyższonymi uprawnieniami.

`undo` uruchamia `groups.remove`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text` | wymagany | Nazwa grupy. |
| `gid` | `int` | brak | Identyfikator liczbowy; system wybiera go, gdy pominięto. |
| `system` | `bool` | `false` | Tworzy grupę systemową. |

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
| External use | `system_program`: `groupadd`, `dseditgroup`, `net`; Grupy są tworzone przez groupadd w systemie Linux, dseditgroup w systemie macOS i net localgroup w systemie Windows. |

## Przykłady {#examples}

Utwórz grupę.

```nurt example
groups.create "ops" privilege: elevated
```

Utwórz grupę z identyfikatorem.

```nurt example
groups.create "ops" gid: 3000 privilege: elevated
```

Utwórz grupę systemową.

```nurt example
groups.create "svc" system: true privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_EXISTS` | Grupa istnieje. | Użyj `groups.add_member`. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wskazany w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`groups.remove`, `groups.add_member`, `accounts`.
