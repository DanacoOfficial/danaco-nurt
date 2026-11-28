---
command: groups.add_member
group: administration
summary: Dodaje konto do grupy.
since: 0.1.1
---
<!-- Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved. -->
<!-- Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl -->
<!-- https://github.com/DanacoOfficial | support@danaco-group.pl -->

# groups.add_member

## Składnia {#synopsis}

```text
groups.add_member <name> <member>
```

## Opis {#description}

`groups.add_member` dodaje konto do grupy poleceniem `gpasswd -a` w systemie Linux, `dseditgroup` w systemie macOS i `net localgroup /add` w systemie Windows. Konto, które już należy do grupy, jest raportowane jako `unchanged`.

Uprawnienia: polecenie akceptuje `privilege`; zapisz `privilege: elevated`, gdy konto uruchamiające Nurt nie jest administratorem. Program, który kończy się niepowodzeniem z powodu braku uprawnień, kończy polecenie błędem `E_PERMISSION`, a `privilege: auto` powtarza je jednokrotnie z podwyższonymi uprawnieniami. Wpis wykonany z podwyższonymi uprawnieniami jest cofany przez ten sam proces z podwyższonymi uprawnieniami.

`undo` uruchamia `groups.remove_member`.

## Parametry {#parameters}

| Parametr | Typ | Wartość domyślna | Opis |
|---|---|---|---|
| `name` | `text` | wymagany | Nazwa grupy. |
| `member` | `text` | wymagany | Konto do dodania. |

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
| External use | `system_program`: `gpasswd`, `dseditgroup`, `net`; Członkowie są dodawani przez gpasswd w systemie Linux, dseditgroup w systemie macOS i net localgroup w systemie Windows. |

## Przykłady {#examples}

Dodaj konto do grupy.

```nurt example
groups.add_member "docker" "deploy" privilege: elevated
```

Dodaj konto wskazane parametrem.

```nurt example
groups.add_member name: "ops" member: "ann" privilege: elevated
```

Wykonaj zmianę z podwyższonymi uprawnieniami; dodanie parametru wywołania `preview: true` najpierw pokazuje skutek.

```nurt example
groups.add_member "ops" "ann" privilege: elevated
```

## Błędy {#errors}

| Kod | Znaczenie | Typowa poprawka |
|---|---|---|
| `E_NOT_FOUND` | Grupa nie istnieje. | Utwórz ją poleceniem `groups.create`. |
| `E_PERMISSION` | Konto nie ma uprawnień administratora. | Dodaj `privilege: elevated`. |
| `E_EXTERNAL_MISSING` | Program systemowy polecenia nie jest zainstalowany. | Zainstaluj program wskazany w komunikacie. |
| `E_STAGE_FAILED` | Program systemowy zakończył się błędem. | Przeczytaj koniec wyjścia błędów w komunikacie. |

## Powiązane {#related}

`groups.remove_member`, `groups.create`.
